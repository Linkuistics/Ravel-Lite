### Session 2 (2026-04-18T05:31:05Z) — config overlays + pi MEMORY_DIR fix

- **Task 2 (*.local.yaml overlay):** Implemented `merge_yaml()` and `load_with_optional_overlay<T>()` in `src/config.rs`. All three public loaders (`load_shared_config`, `load_agent_config`, `load_tokens`) now route through the generic helper. Deep-merge semantics: scalar collisions go to overlay, map collisions recurse, so `models.work: ""` in an overlay blanks only that key without losing `models.reflect` / `models.dream`. Eight new unit tests cover the merge primitive, each loader, and an error-path test that confirms the overlay file path is named in deserialization failures. Architecture doc updated with overlay diagram and an operator recipe in the Configuration section.

- **Task 3 ({{MEMORY_DIR}} hard-error):** Replaced all three `{{MEMORY_DIR}}` occurrences in `defaults/agents/pi/prompts/memory-prompt.md` with `{{PLAN}}/auto-memory`. Rewrote `PiAgent::load_prompt_file` to delegate to `crate::prompt::substitute_tokens` instead of doing ad-hoc `str::replace` — the old code bypassed the hard-error guard regex entirely, which was the root cause of the silent pass-through. Three new tests: happy-path substitution, regression guard (dangling token must fail), and a drift-guard (`shipped_pi_prompts_have_no_dangling_tokens`) that iterates every on-disk pi prompt.

- **What worked:** Both tasks implemented cleanly with full test coverage. `cargo test` passed (143 lib + 8 integration tests for the overlay feature; 135 lib + 8 integration for the pi prompt fix).

- **What to try next:** Task 4 (capture pi stderr on non-zero exit) is the last pi-path correctness bug and is now unblocked. Task 5 (bump pi default models) and Task 6 (extend `embedded_defaults_are_valid` for pi) are also unblocked and independent of each other.

- **Key learnings:** The `{{MEMORY_DIR}}` bug persisted because `load_prompt_file` did its own `str::replace` and never ran the guard regex enforced by `substitute_tokens`. Unifying all prompt-loading through a single substitution path is the correct fix — drift guards only work when there is one canonical substitution path to enforce them against.
