# Manual Release and TestAnyware Capture Tooling — Design

**Date:** 2026-04-27
**Status:** Approved (user signoff in brainstorming session)
**Implements backlog items:**

- `provision-homebrew-tap-infrastructure-and-cut-first-test-release`
- `integrate-testanyware-vm-for-capturing-interactive-ravel-lite-commands`
- `set-up-asciidoc-to-html-build-pipeline-for-ravel-lite-docs` (cleanup
  only — work landed in earlier session, backlog status was stale)

## Context

Two infrastructure tasks land in the same work phase. They are
independent but share one sequencing dependency: the TestAnyware capture
script invokes `brew install linkuistics/taps/ravel-lite`, which
requires the Homebrew tap formula to be live. End-to-end execution of
the capture script therefore waits for the operator to cut a tagged
release and run the new release scripts.

## Part 1 — Manual Homebrew tap deployment

### Decision

Retire the `cargo-dist` GitHub Actions release pipeline. Replace it
with two local scripts that produce per-target tarballs, render a
hand-rolled Homebrew formula, and publish to a sibling clone of
`Linkuistics/homebrew-taps`.

### Rationale

- Operator wants visible, controllable releases without delegating to CI.
- `cargo-dist`'s value (cross-compile orchestration, formula
  generation, GitHub Release upload) is replaceable by a small,
  readable shell script.
- Keeping `cargo-dist` half-plumbed (CI removed, metadata kept) leaves
  a cognitively muddled state. Cleaner to remove fully.

### Files removed

- `.github/workflows/release.yml`
- `[workspace.metadata.dist]` block from `Cargo.toml`
- `[profile.dist]` block from `Cargo.toml` (cargo-zigbuild uses the
  standard `[profile.release]`; the parallel profile no longer earns
  its keep)

### Files added

- `scripts/release-build.sh` — produces tarballs, sha256s, and a
  rendered formula in `target/dist/`.
- `scripts/release-publish.sh` — creates the GitHub Release, copies
  the formula into the tap clone, commits, pushes.
- `scripts/templates/ravel-lite.rb.tmpl` — Homebrew formula template
  with `@VERSION@` / `@SHA_*@` placeholders. Structured with
  `on_macos` / `on_linux` × `on_arm` / `on_intel` blocks pointing at
  per-arch tarballs from the GitHub Release.

### Files modified

- `README.md` — `Releasing` and `Release pipeline (dist) prerequisites`
  sections rewritten for the manual flow. cargo-dist references deleted.
- `.gitignore` — verify `target/` already covers `target/dist/`.

### Tooling prerequisites (one-time, documented in README)

- `brew install zig`
- `cargo install cargo-zigbuild`
- `rustup target add x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu`
- `gh` CLI authenticated to GitHub.
- Sibling clone of `Linkuistics/homebrew-taps` at the path named by
  `$RAVEL_TAP_DIR` (defaults to `~/Development/homebrew-taps`).

### `release-build.sh` flow

1. Read version from `git describe --tags --abbrev=0`. Refuse to run on
   a dirty tree or a non-tag commit.
2. Clean `target/dist/`.
3. For each of 4 targets:
   - `aarch64-apple-darwin`, `x86_64-apple-darwin`: native
     `cargo build --release --target $T`.
   - `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`:
     `cargo zigbuild --release --target $T.2.17` (glibc 2.17 floor for
     wide compatibility).
   - Stage binary + LICENSE + README into `target/dist/staging/$T/`.
   - Tar+xz to `target/dist/ravel-lite-v$VER-$T.tar.xz`.
4. Compute sha256 for each tarball.
5. Render `ravel-lite.rb` from template via `sed` substitution.
6. Print summary; remind operator to inspect `target/dist/` before
   running `release-publish.sh`.

### `release-publish.sh` flow

1. Sanity checks: artifacts present in `target/dist/`, version matches
   current tag, `gh auth status` returns 0, `$RAVEL_TAP_DIR` is a clean
   git repo on `main`.
2. `gh release create v$VER --title "Release v$VER" --notes "..." target/dist/*.tar.xz`.
3. Copy `target/dist/ravel-lite.rb` → `$RAVEL_TAP_DIR/Formula/ravel-lite.rb`.
4. `cd $RAVEL_TAP_DIR && git add Formula/ravel-lite.rb && git commit -m "ravel-lite v$VER" && git push`.

### Release notes source

Empty for now (no `CHANGELOG.md` exists). Revisit when changelog
authoring becomes part of the release ritual.

### `cargo-release` is unchanged

Still the version-bump + tag tool. The new full release flow:

```
cargo release patch --execute
git push origin main --follow-tags
./scripts/release-build.sh        # inspect target/dist/
./scripts/release-publish.sh
```

## Part 2 — TestAnyware capture script

### Decision

Land one concrete bash script that drives the ravel-lite tutorial
scenario inside a TestAnyware macOS VM. Treat it as the de-facto
template for an LLM to generate sibling scripts for other
documentation pages.

### Rationale

- User intends this script to be one of many; an LLM will generate
  scenario-specific scripts on demand. The template shape matters.
- A YAML-driven generic runner would be premature abstraction — better
  to let the right schema emerge from 2-3 concrete scripts.
- Backlog item explicitly cautions against rabbit-holing — concrete
  scenario script is the smallest useful slice that satisfies "integrate".

### Files added

- `scripts/capture/ravel-lite-tutorial.sh`

### Files modified

- `.gitignore` — add `docs/captures/`. Tutorial author selectively
  `git add`s the screenshots that ship in chapters; raw run output is
  ignored.

### Output destinations

- `docs/captures/ravel-lite-tutorial/state/` — the `LLM_STATE/` tree
  pulled out of the VM.
- `docs/captures/ravel-lite-tutorial/screens/` — PNG screenshots.

### Script section structure

The script is organised into eight named sections, each emitting a
`[STEP-NAME] message` log line so an LLM driver can correlate
captures and outputs to script phases:

```
1. PREFLIGHT       — testanyware/gh/jq on PATH; warn if formula not yet at tap
2. VM_LIFECYCLE    — vm start --platform macos --display 1920x1080;
                     trap EXIT → vm stop (no orphans)
3. INSTALL         — testanyware exec → brew install linkuistics/taps/ravel-lite
4. SCENARIO_INPUT  — mkdir example dir; ravel-lite init
5. SCENARIO_RUN    — drive ravel-lite create + run interactively
6. CAPTURE_SCREENS — agent snapshot --window "Terminal" + screenshot at scripted moments
7. CAPTURE_STATE   — testanyware download VM state path → docs/captures/.../state/
8. TEARDOWN        — implicit via EXIT trap
```

### Choreography primitives

- `testanyware find-text --timeout 30 "phase: work"` — anchor wait on
  TUI text. Strongly preferred over `sleep`.
- `testanyware agent snapshot --window "Terminal"` — accessibility-tree
  snapshot for crisp window-only screenshots.
- `testanyware input type` / `testanyware input key return` — drive
  interactive prompts.
- `testanyware exec` — bypass the keyboard for non-interactive shell
  commands (faster and more reliable).

### Sequencing dependency on Part 1

The `INSTALL` section's `brew install linkuistics/taps/ravel-lite`
will fail until the operator publishes the formula via Part 1's
`release-build.sh` + `release-publish.sh`. For this work phase:

- Capture script lands as code with structure validated (shellcheck,
  preflight checks).
- End-to-end execution is deferred until the operator cuts a tagged
  release. That is a sequencing artifact, not a script defect.

### No ravel-lite-side code changes

Pure additions: one shell script, one `.gitignore` entry. Nothing
under `src/`, `Cargo.toml`, etc.

## Part 3 — Asciidoc backlog cleanup

The backlog item
`set-up-asciidoc-to-html-build-pipeline-for-ravel-lite-docs` is stale
— the pipeline already lives on disk:

- `scripts/build-docs.sh`
- `docs/build-config.sh`
- `docs/manifest.txt`
- `docs/templates/`

Action: flip the task to `done` with a `Results:` block pointing at
the landed work. No code changes.
