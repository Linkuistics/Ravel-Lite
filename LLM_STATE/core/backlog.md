# Backlog

## Tasks

### Narrow `warn_if_project_tree_dirty` to work-agent-touched files only

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`warn_if_project_tree_dirty` at `phase_loop.rs:94` is pathspec-unscoped
— it fires on any dirty file in the project tree. In monorepos with
multiple plans the check can still produce false positives from sibling
plans' in-flight writes, even after the atomic phase-transition fix.

Narrow the check to: compute `git diff --name-only <work_baseline>`
(files changed since the work baseline) intersected with the current
dirty list, so the warning only fires on files the active work agent
could plausibly have touched. This is a defense-in-depth refinement;
no correctness regression possible since the current check is strictly
more noisy, never more accurate.

**Results:** _pending_

---

### Make `ravel-lite survey` plan roots positional args instead of `--root` options

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none

**Description:**

The `survey` subcommand previously required each plan root to be passed
via a repeated `--root <path>` option (`required = true`). Roots are the
subcommand's primary input, so positional variadic args are the
idiomatic POSIX surface.

**Results:**

- `src/main.rs`: replaced `#[arg(long, required = true)] root: Vec<PathBuf>`
  with `#[arg(required = true, num_args = 1..)] roots: Vec<PathBuf>`,
  renaming the field to match the new plural-positional semantics.
  Match-arm destructure updated accordingly.
- `src/survey/invoke.rs`: updated docstring and the "no plans
  discovered" error message to refer to "plan root" instead of
  `--root`.
- `src/survey/discover.rs`: updated two `--root` references in
  doc/test comments to "plan root" / "plan-root argument".
- `tests/integration.rs`: updated one comment reference.
- `docs/architecture.md`: updated the `### ravel-lite survey ...`
  usage signature and surrounding prose to the new positional form.
- Verification: `grep --root` returns zero matches; `cargo build`
  clean; `./target/debug/ravel-lite survey --help` shows
  `Usage: ravel-lite survey [OPTIONS] <ROOTS>...`; invoking with no
  roots fails with clap's standard "required arguments were not
  provided" error; full test suite passes (303 tests, 0 failures).
- No `discover_plans` signature change was needed — it already takes
  `&Path`, so only the CLI surface and its prose references moved.

---
