#!/usr/bin/env bash
#
# Run the local hygiene gate. Currently: clippy with warnings-as-errors
# across all targets in the workspace. Run before pushing to main, or
# install the pre-push hook once with:
#
#   ./scripts/install-hooks.sh
#
# Designed to grow: add `cargo fmt --check`, `cargo test`, etc. here as
# the project's gating policy expands. There is no CI — this script is
# the single source of truth for "what must pass before main".

set -euo pipefail
IFS=$'\n\t'
trap 'echo "check: error on line $LINENO" >&2' ERR

cargo clippy --all-targets --workspace -- -D warnings

# Reject untagged bail!/anyhow!/ensure! macros under src/**.
#
# The convention (src/cli/error_context.rs) is that every fallible
# call-site attaches an ErrorCode — via `bail_with!(ErrorCode::X, ...)`
# for early-exit, or `.with_code(ErrorCode::X)` for ?-propagation. The
# `\b(anyhow|bail|ensure)!\(` pattern catches `anyhow::bail!(`,
# `anyhow::anyhow!(`, bare `bail!(`, etc., but not `bail_with!(` (the `_`
# breaks the word match). Lines that are legitimately untagged carry an
# inline `// errorcode-exempt: <reason>` marker; the guard ignores them.
violations=$(rg --line-number --pcre2 \
  '\b(anyhow|bail|ensure)!\(' \
  --glob 'src/**/*.rs' \
  | { grep -v 'errorcode-exempt:' || true; })

if [[ -n "$violations" ]]; then
  echo "check: untagged bail!/anyhow!/ensure! macros found under src/**." >&2
  echo "       Use bail_with!(ErrorCode::X, ...) or chain .with_code(ErrorCode::X)" >&2
  echo "       on the propagated error. For unavoidable cases (helpers tagged" >&2
  echo "       downstream, tests asserting the untagged path), append" >&2
  echo "       '// errorcode-exempt: <reason>' to the line." >&2
  echo >&2
  printf '%s\n' "$violations" >&2
  exit 1
fi
