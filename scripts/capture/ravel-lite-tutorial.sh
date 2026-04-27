#!/usr/bin/env bash
#
# Capture the ravel-lite tutorial scenario inside a TestAnyware macOS VM.
#
# Outputs:
#   docs/captures/ravel-lite-tutorial/state/    — pulled LLM_STATE/ tree
#   docs/captures/ravel-lite-tutorial/screens/  — PNG screenshots
#
# Prerequisite: ravel-lite formula must be live in Linkuistics/homebrew-taps
# (see scripts/release-build.sh + scripts/release-publish.sh).
#
# Section markers ([STEP-NAME]) on each log line let an LLM driver
# correlate captures and outputs to script phases.

set -euo pipefail
IFS=$'\n\t'

readonly REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly CAPTURE_DIR="$REPO_ROOT/docs/captures/ravel-lite-tutorial"
readonly STATE_DIR="$CAPTURE_DIR/state"
readonly SCREENS_DIR="$CAPTURE_DIR/screens"

# Default macOS VM user. Override via env if your golden image differs.
readonly VM_USER="${VM_USER:-tester}"
readonly EXAMPLE_DIR="/Users/${VM_USER}/Development/ravel-tutorial-example"

# Endpoints discovered by vm_lifecycle_start; cleared on cleanup.
VM_ID=""
VNC_ENDPOINT=""
AGENT_ENDPOINT=""

log() { echo "[$1] ${*:2}"; }
die() { echo "capture: $*" >&2; exit 1; }

cleanup() {
  if [[ -n "$VM_ID" ]]; then
    log TEARDOWN "stopping VM $VM_ID"
    testanyware vm stop "$VM_ID" || true
  fi
}
trap cleanup EXIT

preflight() {
  log PREFLIGHT "checking dependencies"
  command -v testanyware >/dev/null || die "testanyware not on PATH"
  command -v jq >/dev/null || die "jq not on PATH"
  mkdir -p "$STATE_DIR" "$SCREENS_DIR"
}

vm_lifecycle_start() {
  log VM_LIFECYCLE "starting macOS VM (1920x1080)"
  VM_ID="$(testanyware vm start --platform macos --display 1920x1080 | tail -n1)"
  [[ -n "$VM_ID" ]] || die "vm start did not return a VM id"
  log VM_LIFECYCLE "VM_ID=$VM_ID"

  # Endpoint discovery: this assumes 'testanyware vm list --format json'
  # emits records with .id/.vnc/.agent fields. The exact subcommand
  # surface needs validation against a live testanyware on first run;
  # if it differs, override VNC_ENDPOINT and AGENT_ENDPOINT via env vars
  # and short-circuit this block.
  local info
  info="$(testanyware vm list --format json | jq -r --arg id "$VM_ID" '.[] | select(.id == $id)')"
  VNC_ENDPOINT="${VNC_ENDPOINT:-$(echo "$info" | jq -r '.vnc')}"
  AGENT_ENDPOINT="${AGENT_ENDPOINT:-$(echo "$info" | jq -r '.agent')}"
  [[ -n "$VNC_ENDPOINT" && -n "$AGENT_ENDPOINT" ]] \
    || die "could not discover vnc/agent endpoints for $VM_ID"
  log VM_LIFECYCLE "vnc=$VNC_ENDPOINT agent=$AGENT_ENDPOINT"
}

install_ravel_lite() {
  log INSTALL "brew install linkuistics/taps/ravel-lite"
  testanyware exec --agent "$AGENT_ENDPOINT" \
    "brew tap linkuistics/taps && brew install ravel-lite"
  testanyware exec --agent "$AGENT_ENDPOINT" "ravel-lite --version"
}

scenario_input() {
  log SCENARIO_INPUT "preparing example project at $EXAMPLE_DIR"
  testanyware exec --agent "$AGENT_ENDPOINT" \
    "mkdir -p ${EXAMPLE_DIR} && cd ${EXAMPLE_DIR} && ravel-lite init"
}

screenshot_at() {
  local label="$1"
  log CAPTURE_SCREENS "screenshot $label"
  testanyware screenshot --vnc "$VNC_ENDPOINT" -o "$SCREENS_DIR/${label}.png"
}

scenario_run() {
  log SCENARIO_RUN "opening Terminal in VM"
  testanyware exec --agent "$AGENT_ENDPOINT" "open -a Terminal"
  testanyware find-text --vnc "$VNC_ENDPOINT" "\$" --timeout 15 >/dev/null

  log SCENARIO_RUN "driving 'ravel-lite create'"
  testanyware input type --vnc "$VNC_ENDPOINT" \
    "cd ${EXAMPLE_DIR} && ravel-lite create"
  testanyware input key --vnc "$VNC_ENDPOINT" return
  testanyware find-text --vnc "$VNC_ENDPOINT" "plan name" --timeout 15 >/dev/null
  screenshot_at "01-create-plan-name-prompt"
  testanyware input type --vnc "$VNC_ENDPOINT" "core"
  testanyware input key --vnc "$VNC_ENDPOINT" return

  log SCENARIO_RUN "driving 'ravel-lite run'"
  testanyware input type --vnc "$VNC_ENDPOINT" "ravel-lite run core"
  testanyware input key --vnc "$VNC_ENDPOINT" return
  testanyware find-text --vnc "$VNC_ENDPOINT" "phase: work" --timeout 30 >/dev/null
  screenshot_at "02-tui-phase-work"
}

capture_state() {
  log CAPTURE_STATE "pulling LLM_STATE/ from VM"
  testanyware download --agent "$AGENT_ENDPOINT" \
    "${EXAMPLE_DIR}/LLM_STATE" "$STATE_DIR"
}

main() {
  preflight
  vm_lifecycle_start
  install_ravel_lite
  scenario_input
  scenario_run
  capture_state
  log MAIN "capture complete; outputs in $CAPTURE_DIR"
}

main "$@"
