# Memory

## Pi agent has unresolved `{{MEMORY_DIR}}` token
`substitute_tokens` now hard-errors on unresolved tokens, so pi invocation will fail loudly rather than pass the literal token through to output.

## Pi scope meta-task blocks all pi-specific bug work
A meta decision task must be resolved before investing in pi bugs (stderr capture, integration tests, model update).

## Pi subagent definitions live at `agents/pi/subagents/`
`defaults/agents/pi/subagents/` holds pi subagent definitions (brainstorming, tdd, writing-plans). The former `defaults/skills/` location was a misnomer; `init.rs` embed paths and `pi.rs` reads are updated accordingly.

## `init.rs` drift-detection test guards coding-style registration
The test reads `defaults/fixed-memory/coding-style-*.md` at test time and asserts every file on disk is registered as an `EmbeddedFile`. Adding a new coding-style file without registering it fails the test.

## `embedded_defaults_are_valid` test asserts non-empty model strings
Every (agent, phase) pair in `defaults/agents/claude-code/config.yaml` must have a non-empty model string. The test catches model omissions that would silently delegate model selection to the spawn context.

## `warn_if_project_tree_dirty` fires after `GitCommitWork`
`git::working_tree_status` checks the project tree post-commit; a dirty tree logs a `⚠  WARNING` to the TUI. Guards against sessions that commit only meta files and leave source changes unstaged.

## `StreamLineOutcome` enum distinguishes ignored vs malformed stream lines
Replacing `Option<FormattedOutput>` with an enum makes `valid but no display` and `parse failure` distinguishable. Apply this pattern wherever an `Option` return collapses two semantically distinct outcomes into one.

## Survey stdout read has 300s timeout
`src/survey/invoke.rs` wraps the stdout read in `tokio::time::timeout` (`DEFAULT_SURVEY_TIMEOUT_SECS = 300`); on expiry the child is killed and the error includes elapsed time, captured bytes, partial stdout, and remediations. Override via `--timeout-secs`.

## Work-phase prompt lacks done-marking instruction
The work-phase prompt never instructs the agent to flip `Status:` lines from pending to done, causing completed backlog items to appear silently stale.

## Phase contract test validates per-phase file writes
`phase_contract_round_trip_writes_expected_files` runs `phase_loop` from `analyse-work` via `ContractMockAgent`; 6 assertions cover latest-session.md, commit-message.md consumed, memory.md updated, backlog.md updated, phase.md ends at `work`, and git log subjects.
