#!/usr/bin/env bash
#
# Publish the artifacts produced by release-build.sh:
#   1. Create a GitHub Release on Linkuistics/Ravel-Lite for v<ver> and
#      upload all tarballs from target/dist/.
#   2. Copy ravel-lite.rb into $RAVEL_TAP_DIR/Formula/, commit, push.
#
# Prerequisite: ./scripts/release-build.sh has just run successfully.
# Env: RAVEL_TAP_DIR (default ~/Development/homebrew-taps).

set -euo pipefail
IFS=$'\n\t'

readonly REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly DIST_DIR="$REPO_ROOT/target/dist"
readonly TAP_DIR="${RAVEL_TAP_DIR:-$HOME/Development/homebrew-taps}"

die() {
  echo "release-publish: $*" >&2
  exit 1
}

preflight() {
  command -v gh >/dev/null || die "gh CLI not on PATH"
  gh auth status >/dev/null 2>&1 || die "gh not authenticated; run 'gh auth login'"
  [[ -d "$DIST_DIR" ]] || die "no $DIST_DIR; run scripts/release-build.sh first"
  [[ -f "$DIST_DIR/ravel-lite.rb" ]] || die "no rendered formula at $DIST_DIR/ravel-lite.rb"
  compgen -G "$DIST_DIR/*.tar.xz" >/dev/null || die "no tarballs in $DIST_DIR"
  [[ -d "$TAP_DIR/.git" ]] || die "tap clone not found at $TAP_DIR (set RAVEL_TAP_DIR)"
}

read_version() {
  git -C "$REPO_ROOT" describe --tags --abbrev=0 | sed 's/^v//'
}

verify_tag_matches_artifacts() {
  local version="$1"
  local sample
  sample="$(ls "$DIST_DIR"/ravel-lite-v*-aarch64-apple-darwin.tar.xz 2>/dev/null | head -n1)" \
    || die "missing aarch64-apple-darwin tarball"
  [[ "$sample" == *"ravel-lite-v${version}-"* ]] \
    || die "artifact version mismatch: $sample does not contain v${version}"
}

create_github_release() {
  local version="$1"
  local tag="v${version}"
  echo "release-publish: creating GitHub Release $tag"
  gh release create "$tag" \
    --repo Linkuistics/Ravel-Lite \
    --title "Release $tag" \
    --notes "Release $tag" \
    "$DIST_DIR"/*.tar.xz
}

push_formula_to_tap() {
  local version="$1"
  echo "release-publish: pushing formula to $TAP_DIR"
  mkdir -p "$TAP_DIR/Formula"
  cp "$DIST_DIR/ravel-lite.rb" "$TAP_DIR/Formula/ravel-lite.rb"
  git -C "$TAP_DIR" add Formula/ravel-lite.rb
  git -C "$TAP_DIR" commit -m "ravel-lite v${version}"
  git -C "$TAP_DIR" push
}

main() {
  preflight
  local version
  version="$(read_version)"
  verify_tag_matches_artifacts "$version"

  create_github_release "$version"
  push_formula_to_tap "$version"

  echo
  echo "release-publish: done. Verify with:"
  echo "  brew update && brew install linkuistics/taps/ravel-lite"
}

main "$@"
