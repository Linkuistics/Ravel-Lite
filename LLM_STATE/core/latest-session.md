### Session 3 (2026-04-20T05:39:05Z) — rename to ravel-lite + survey positional args

- Completed a full project rename from `raveloop`/`Raveloop`/`Mnemosyne`/`LLM_CONTEXT_PI`
  to `ravel-lite`/`Ravel-Lite`/`Ravel` across all source files, tests, docs, defaults,
  Cargo.toml, Cargo.lock, .gitignore, and README (5 commits covering Cargo package name,
  env-var and config-dir paths, prose renames, test fixture project names, and straggler
  defaults files).
- Completed the "Make `ravel-lite survey` plan roots positional args" backlog task:
  `#[arg(long, required = true)] root: Vec<PathBuf>` replaced with
  `#[arg(required = true, num_args = 1..)] roots: Vec<PathBuf>` in `src/main.rs`;
  help text, error messages, doc comments in `src/survey/invoke.rs`,
  `src/survey/discover.rs`, `tests/integration.rs`, and `docs/architecture.md` all
  updated to match the new positional surface.
- Full test suite (303 tests) passes; `cargo build` clean; `--help` output verified.
- The "Narrow `warn_if_project_tree_dirty`" task remains `not_started` — it is the
  only remaining backlog item and the natural candidate for the next session.
