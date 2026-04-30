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
