# Ravel-Lite

> An orchestration loop for LLM development cycles.
> Compose. Reflect. Dream. Triage. Repeat.

Multi-agent orchestrator for backlog-driven LLM development. Supports
[Claude Code](https://claude.ai/code) and
[Pi](https://github.com/mariozechner/pi-coding-agent) as selectable
agent backends, with shared parameterized phase files and pluggable
configuration.

A single Rust binary (`ravel-lite`) with a
[Ratatui](https://ratatui.rs) TUI. See
[docs/architecture.md](docs/architecture.md) for the module layout,
message model, and agent trait.

## Phase Cycle

```
work → analyse-work → git-commit-work →
reflect → git-commit-reflect → [dream trigger?] →
dream → git-commit-dream →
triage → git-commit-triage → [continue?] → work
```

- **work** (interactive) — user steers task selection, implements task
- **analyse-work** (headless) — examines git diff, produces session
  log and commit message from ground truth
- **reflect** (headless) — distils learnings into durable memory
- **dream** (headless, conditional) — lossless memory rewrite when
  word count exceeds baseline + headroom
- **triage** (headless) — adjusts backlog, dispatches subagents for
  cross-plan propagation
- **git-commit-\*** — per-phase audit trail commits

## Principles

- **No magic.** All config, prompts, phase state, and memory are
  readable files on disk. Nothing is embedded or hidden.
- **Visible, auditable, adjustable.** Every input is a file the user
  can inspect and edit. Every state transition writes to the filesystem.
- **Agents are subprocesses.** The orchestrator spawns CLI processes,
  reads their JSON stream output, and renders progress. It never calls
  LLM APIs directly.

## Project layout

A plan always lives at `<project>/<state-dir>/<plan>` — typically
`<project>/LLM_STATE/<plan>`. Ravel-lite derives the "project" (the
subtree it controls) from this layout as `<plan_dir>/../..` — pure
path math, independent of where `.git` sits.

### Monorepo subtrees

Because the project root is derived from the plan path rather than
from `.git`, ravel-lite runs cleanly inside a monorepo subtree. Place
the plan at `<monorepo>/<path-to-subtree>/LLM_STATE/<plan>` and
ravel-lite will treat `<path-to-subtree>` as the project: all git
queries (dirty-tree checks, baseline diffs, snapshot for analyse-work)
are pathspec-scoped to the subtree, so sibling subtrees in the same
monorepo are invisible. Baseline SHAs remain repo-wide — the scope is
on the query, not the anchor.

Commit-message prefix conventions (e.g., conventional-commits rules
the monorepo enforces) are not automated — the per-commit
`commit-message.md` authored by analyse-work is the customisation
point.

## Documentation

User-facing docs are authored as AsciiDoc chapters under `docs/` and
built into HTML pages wrapped in the Linkuistics site chrome. Chapter
order lives in `docs/manifest.txt` (one source path per line); the HTML
shell is `docs/templates/page-shell.html`; per-project values (project
slug, back-link, output directory) live in `docs/build-config.sh`.

Prerequisites: `asciidoctor` on `PATH` (`brew install asciidoctor` on
macOS). Build:

```bash
./scripts/build-docs.sh
```

The script flattens `tutorial/foo.adoc` and `reference/bar.adoc` into
a single `projects/ravel-lite/tutorial-foo.html` /
`projects/ravel-lite/reference-bar.html` layer under the configured
`OUTPUT_DIR` — by default the sibling `www.linkuistics.com` checkout —
and injects a prev/next chapter navigation at the bottom of each page
based on manifest order.

The template and build script are intentionally project-agnostic: to
adopt the same pattern in another project, copy `scripts/build-docs.sh`
and `docs/templates/` verbatim and edit only `docs/build-config.sh`.

## Releasing

Releases are cut by hand from a developer machine — no CI is involved.
Versions are bumped with [`cargo-release`](https://github.com/crate-ci/cargo-release);
two local scripts then produce per-target tarballs and update the
`Linkuistics/homebrew-taps` Homebrew tap so users can
`brew install linkuistics/taps/ravel-lite` to get a bottled binary
(no Rust toolchain required on their machine).

Full release flow:

```bash
cargo release patch --execute        # 0.1.0 → 0.1.1, commit + tag v0.1.1
git push origin main --follow-tags
./scripts/release-build.sh           # build 4 tarballs + render formula
                                     # → target/dist/  (inspect before publishing)
./scripts/release-publish.sh         # gh release create + push formula to tap
```

Omit `--execute` on `cargo release` for a dry run. Push is intentionally
separate so the release commit and tag can be inspected locally first;
`release-publish.sh` is likewise separate from `release-build.sh` so the
artifacts in `target/dist/` can be inspected before any remote
side-effect. `cargo-release` configuration lives in `release.toml`.

`ravel-lite version` and `ravel-lite --version` print the crate version
plus `git describe --tags --always --dirty` and the UTC build timestamp,
so a running binary always identifies the commit it was built from.

### Release tooling prerequisites (one-time)

The release scripts assume a macOS workstation with the following set up:

```bash
cargo install cargo-release cargo-zigbuild
brew install zig
rustup target add x86_64-apple-darwin \
    aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu
gh auth login                        # GitHub CLI authenticated
```

The Linux targets are cross-compiled from macOS via `cargo zigbuild`
(zig is the linker), avoiding a docker/QEMU dependency. Linux glibc is
pinned to 2.17 for wide compatibility (RHEL 7-era).

The publish step expects a sibling clone of
[`Linkuistics/homebrew-taps`](https://github.com/Linkuistics/homebrew-taps)
on the same machine. By default it is looked up at
`~/Development/homebrew-taps`; override with `RAVEL_TAP_DIR=/some/path`.

The Homebrew formula is rendered from
`scripts/templates/ravel-lite.rb.tmpl` by `release-build.sh` — edit the
template to change formula structure, then run a fresh release to
regenerate.

## License

See [LICENSE](LICENSE).
