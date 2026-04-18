### Session 1 (2026-04-18T07:39:42Z) — Atomic phase transitions in script-phase handlers

- Worked on "Commit plan-state transitions atomically with their script phases" (bugfix)
- Fixed all four `ScriptPhase::GitCommit*` handlers in `src/phase_loop.rs` to call `write_phase(next)` before `git_commit_plan`, ensuring phase.md is captured in the same commit as other plan-state writes and the tree is clean at user-prompt points
- `GitCommitTriage` additionally calls `git_save_work_baseline` pre-commit so work-baseline is atomically captured; `LlmPhase::Work` entry reduced to a first-run fallback that seeds work-baseline only when the file is missing
- `latest-session.md` deletion from `LlmPhase::Work` entry was dropped: analyse-work overwrites it unconditionally, so the deletion was decorative; leaving it in place preserves the prior session log for operator inspection
- `GitCommitReflect` restructured to hoist the `should_dream` check so the phase write happens before the commit while the skip banner still renders correctly
- Added two integration tests in `tests/integration.rs`: `git_commit_triage_leaves_plan_tree_clean_at_user_prompt` and `git_commit_work_leaves_plan_tree_clean_at_user_prompt` — both assert `git status --porcelain -- <plan_dir>` is empty after `phase_loop` returns from a user-declined exit; both were RED before the fix and GREEN after
- Full suite: 10/10 integration, 144/144 unit, zero new clippy warnings
- Next: add pi agent integration test (next backlog task), or extract shared spawn/stream boilerplate to `src/agent/common.rs`
