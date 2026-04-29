#!/usr/bin/env bash
#
# Run the local hygiene gate. Currently: clippy with warnings-as-errors
# across all targets in the workspace. Run before pushing to main, or
# wire into a pre-push hook with:
#
#   echo '#!/usr/bin/env bash\nexec scripts/check.sh' > .git/hooks/pre-push
#   chmod +x .git/hooks/pre-push
#
# Designed to grow: add `cargo fmt --check`, `cargo test`, etc. here as
# the project's gating policy expands. There is no CI — this script is
# the single source of truth for "what must pass before main".

set -euo pipefail
IFS=$'\n\t'
trap 'echo "check: error on line $LINENO" >&2' ERR

cargo clippy --all-targets --workspace -- -D warnings
