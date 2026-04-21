### Session 3 (2026-04-21T12:08:16Z) — Incremental survey via --prior (task 5b)

- Implemented all seven deliverables for task 5b: delta classifier, incremental render, cold/warm invoke split, --prior/--force CLI flags, schema_version guard, noop fast path, and 13 new tests.
- New module `src/survey/delta.rs` introduces `classify_delta`, `merge_delta`, and `DeltaClassification` which drives the cold/incremental decision in `invoke.rs`.
- `src/survey/compose.rs` gained `render_incremental_survey_input` — sends only changed+added plans to the LLM, carrying the full prior as context.
- `src/survey/invoke.rs` refactored: extracted `spawn_claude_and_read` so the cold and incremental paths share identical subprocess/timeout/error logic; added prior-load, classify, merge, and schema-version validation.
- `src/survey/schema.rs` gained `schema_version: u32` with `#[serde(default)]` for forward-compatible YAML parsing of 5a-emitted files without the field.
- `src/main.rs`: `--prior <file>` and `--force` flags added to the `survey` subcommand; forwarded through to `run_survey`.
- `defaults/survey-incremental.md` added as the warm-path prompt; registered in `src/init.rs::EMBEDDED_FILES`.
- Tests: 13 new tests in `src/survey/delta.rs` (unit classify/merge), `src/survey/compose.rs` (incremental render), `src/survey/invoke.rs` (schema-version guard), and `tests/integration.rs` (3 end-to-end tests). Total: 203 library + 20 integration, all green. Clippy clean on touched files; 6 pre-existing `doc_lazy_continuation` warnings on `PlanRow::input_hash` are out-of-scope.
- Noop fast path (`classification.is_noop()`) carries the prior forward with no LLM call — makes 5c's every-cycle survey invocation affordable.
- Gotchas: `Clone` propagation required across all nested `SurveyResponse` structs; `deny(warnings)` surfaced unused-public items when `delta.rs` had no consumer yet.
- Suggests next: task 5c can call `run_survey` with a `--survey-state` path that doubles as `--prior` input and output; `run_single_plan` in `phase_loop.rs` is the dispatch seam.

**Deliberately not committed:** None — all source paths in the snapshot were staged.
