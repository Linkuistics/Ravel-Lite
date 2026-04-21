### Session 4 (2026-04-21T23:13:58Z) — Scope git queries to subtree root for monorepo support

- Implemented the "Make git operations subtree-scoped so ravel-lite can run inside a monorepo" backlog task in full.
- Replaced `find_project_root` (`.git`-walkup) with `project_root_for_plan` — pure path derivation `<plan>/../..`, no disk walk, decoupled from `.git` location.
- Added `-- <project_dir>` pathspec to all three git query functions: `working_tree_status`, `paths_changed_since_baseline`, and `work_tree_snapshot`. `git_commit_plan` intentionally left unchanged (its `git add .` at `plan_dir` CWD is already scoped to plan-state files).
- Updated all four callers: `src/main.rs`, `src/multi_plan.rs`, `src/agent/common.rs`, `src/survey/discover.rs`.
- Added a monorepo scoping integration test (`git_queries_are_scoped_to_subtree_in_monorepo`) that synthesises an outer repo with a sibling subtree and asserts all three query functions see only the ravel-lite subtree's changes.
- Added three `project_root_for_plan` unit tests: correct derivation, shallow-path error, non-existent-path ok (pure math).
- Updated five integration tests in `tests/integration.rs` and one in `src/multi_plan.rs` to use the three-level `<project>/LLM_STATE/<plan>` layout matching ravel-lite convention. Removed the obsolete `load_plan_errors_when_no_git_above_plan` test — the invariant no longer holds under path-math derivation.
- Added README "Project layout" section documenting `<project>/<state-dir>/<plan>` convention and a "Monorepo subtrees" subsection covering pathspec scoping semantics and commit-message-prefix answer.
- All 215 lib tests + 23 integration tests pass. Task marked `done` in backlog with full results, open design-question answers, and verification record.
- What this suggests next: the "Research: expose plan-state markdown as structured data" task is now unblocked and is the natural next candidate — it depends on nothing else and its completion unblocks the task-count extraction task.
