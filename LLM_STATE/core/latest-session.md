### Session 5 (2026-04-21T23:38:12Z) — Continuation-line rendering for dream/triage output

- Implemented `→ …` continuation-line support in `format_result_text` (`src/format.rs`): lines matching `^\s*→\s*(.*)` immediately after an action marker are re-indented to the detail column and styled with the preceding action's intent. Blank lines, insight blocks, and all other non-continuation lines clear the association.
- Added `PROMOTED` and `ARCHIVED` action tags to `ACTION_INTENTS` for triage hand-off markers that emit new backlog tasks or memory entries.
- Updated `defaults/phases/dream.md` output-format spec to describe the new two-line entry layout (label + `→` continuation) so the dream LLM emits output the renderer can align.
- Updated `defaults/phases/work.md` step 10 to allow multiple tasks per session when the user explicitly requests them, while preserving the single-task-per-phase default.
- Five tests added to `src/format.rs`: `PROMOTED`/`ARCHIVED` recognition, continuation alignment, intent inheritance, orphan-arrow fallthrough, and blank-line chain-breaking.
- The triage phase (run before this work session) deleted two tasks: the `done` monorepo subtree-scoping task (cleaned up) and the `not_started` Ravel orchestrator migration task (dropped).

What worked: the `last_action_intent: Option<Option<Intent>>` state variable cleanly threads the preceding action's intent through to continuation lines without adding a new pass over the text. The double-Option encodes "no prior action" (outer None) vs "prior action with no intent" (Some(None)) unambiguously.

What to try next: run the updated dream phase on a real plan to confirm the two-line entries render as intended in the TUI.
