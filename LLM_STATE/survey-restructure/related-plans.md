# Related Plans

## Siblings
Other plans in this project:
- {{DEV_ROOT}}/Ravel-Lite/LLM_STATE/core — main ravel-lite orchestrator backlog. Pre-pivot architectural history (coordinator-plan design, push-plan introduction, stack-based pivot machinery) lives in core's session-log. Code that this plan's items touch (`src/survey/*`, `src/phase_loop.rs`, `src/pivot.rs`, `src/state.rs`, `src/main.rs`) is the same code core works on — so concurrent work in both plans risks merge conflicts until this plan's items are substantially complete.

## Reference
- {{DEV_ROOT}}/Ravel-Lite/docs/survey-pivot-design.md — architectural design doc for this plan. Read first for context; each backlog item references it for scope and rationale.
