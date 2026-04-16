# Configuration for run-plan.sh. Sourced at startup.
#
# Edit this file to override the defaults. Keep variable assignments in
# plain `KEY=value` form — the file is sourced as bash, so it can also
# contain comments (lines starting with `#`) and inline expansions if
# needed.

# -----------------------------------------------------------------------------
# Pi provider and per-phase model selection
# -----------------------------------------------------------------------------
#
# Pi is multi-provider. The provider determines which environment
# variable supplies the API key and which default model namespace
# applies. Each phase below sets a model pattern that pi will match
# against available models (see `pi --list-models`).
#
# Set PHASE_MODEL to the empty string to let pi pick its default (the
# per-settings provider default). The headless trio (reflect, compact,
# triage) defaults to Sonnet 4.6 because those phases are editorial /
# judgment-light and Opus is overkill there. WORK_MODEL is set to Opus
# 4.6 because the work phase is where the engineering happens.
#
# Why not pick one model everywhere:
# - Reflect applies a style guide to a small set of new learnings.
# - Compact rewrites memory.md prose under a strict-lossless contract.
# - Triage adjusts task ordering and emits a propagation list.
# All three are editorial and Sonnet handles them well. Opus is worth
# it for the work phase where model quality moves actual outcomes.

PROVIDER="anthropic"

WORK_MODEL="claude-opus-4-6"
REFLECT_MODEL="claude-sonnet-4-6"
COMPACT_MODEL="claude-sonnet-4-6"
TRIAGE_MODEL="claude-sonnet-4-6"

# -----------------------------------------------------------------------------
# Pi thinking level per phase
# -----------------------------------------------------------------------------
#
# Pi supports thinking levels: off, minimal, low, medium, high, xhigh.
# Setting a thinking level costs tokens and latency but improves
# accuracy on hard tasks. The editorial phases do not need it; the
# work phase benefits from it on complex tasks.
#
# Set to empty string to let pi / the model decide.

WORK_THINKING="medium"
REFLECT_THINKING=""
COMPACT_THINKING=""
TRIAGE_THINKING=""

# -----------------------------------------------------------------------------
# Compaction trigger
# -----------------------------------------------------------------------------
#
# memory.md must grow this many words past `<plan>/compact-baseline`
# before the compact phase fires. Lower = more frequent compaction,
# higher = less. 1500 words is roughly 7–8 cycles at observed growth
# rates. Relative threshold tracks unreflected growth rather than
# absolute size.

HEADROOM=1500
