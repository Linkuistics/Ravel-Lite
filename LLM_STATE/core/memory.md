# Memory

## Pi agent has unresolved `{{MEMORY_DIR}}` token
`substitute_tokens` now hard-errors on unresolved tokens, so pi invocation will fail loudly rather than pass the literal token through to output.

## Pi scope meta-task blocks all pi-specific bug work
A meta decision task must be resolved before investing in pi bugs (stderr capture, integration tests, model update).

## `init.rs` drift-detection test guards coding-style registration
The test reads `defaults/fixed-memory/coding-style-*.md` at test time and asserts every file on disk is registered as an `EmbeddedFile`. Adding a new coding-style file without registering it fails the test.

## `warn_if_project_tree_dirty` fires after `GitCommitWork`
`git::working_tree_status` checks the project tree post-commit; a dirty tree logs a `⚠  WARNING` to the TUI. Guards against sessions that commit only meta files and leave source changes unstaged.

## `StreamLineOutcome` enum distinguishes ignored vs malformed stream lines
Replacing `Option<FormattedOutput>` with an enum makes `valid but no display` and `parse failure` distinguishable. Apply this pattern wherever an `Option` return collapses two semantically distinct outcomes into one.
