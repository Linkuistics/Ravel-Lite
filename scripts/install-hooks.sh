#!/usr/bin/env bash
#
# Install the local git hooks for this repository.
#
# Currently installs a pre-push hook that runs `scripts/check.sh`
# (clippy with warnings-as-errors). Idempotent: re-running detects an
# already-installed hook and no-ops. Refuses to overwrite a foreign
# pre-push hook to avoid clobbering local customisations.

set -euo pipefail
IFS=$'\n\t'
trap 'echo "install-hooks: error on line $LINENO" >&2' ERR

readonly hook_marker='# ravel-lite-hook: pre-push'

main() {
  local repo_root
  repo_root="$(git rev-parse --show-toplevel)"
  cd "$repo_root"

  local hook_path=".git/hooks/pre-push"

  if [[ -e "$hook_path" ]]; then
    if grep -qF "$hook_marker" "$hook_path"; then
      echo "install-hooks: pre-push already installed at $hook_path"
      return 0
    fi
    echo "install-hooks: refusing to overwrite existing $hook_path" >&2
    echo "install-hooks: remove or merge the existing hook, then re-run" >&2
    trap - ERR
    exit 1
  fi

  cat >"$hook_path" <<EOF
#!/usr/bin/env bash
$hook_marker
exec "\$(git rev-parse --show-toplevel)/scripts/check.sh"
EOF
  chmod +x "$hook_path"
  echo "install-hooks: installed pre-push hook at $hook_path"
}

main "$@"
