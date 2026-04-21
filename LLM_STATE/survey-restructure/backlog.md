# Backlog

## Tasks

### Wrap-up — Merge survey-restructure branch, propagate completion, archive plan

**Category:** `housekeeping`
**Status:** `done`
**Dependencies:** 5a ✓, 5b ✓, 5c ✓, 5d ✓ (all survey-restructure deliverables complete)

**Description:**

The survey-restructure plan has delivered all design goals: structured YAML survey (5a),
incremental `--prior` path (5b), multi-plan `run` mode with survey-routed dispatch (5c),
and stack/pivot removal (5d). This task handles close-out:

1. Merge the survey-restructure branch back into main.
2. Propagate the outcome to `core/backlog.md`: note that code-driven survey routing is
   delivered; retire or reframe any core tasks that assumed the old LLM-coordinator model.
3. Decide whether to archive or retire this plan directory. If retired, archive
   `LLM_STATE/survey-restructure` or remove it from active plan routing.

**Results:**

Close-out completed 2026-04-21:

- **Step 1 (merge):** no-op. All survey-restructure work was committed
  directly to `main` across sixteen commits; no feature branch existed.
  Confirmed via `git branch` (only `main` local + `origin/main`) and
  `git log origin/main..HEAD` showing 31 unpushed commits ahead of remote.
  Pushed those to `origin/main` in a single `git push` (new tip
  `11ddece`).
- **Step 2 (propagation):** removed all live-plan pointers from
  `LLM_STATE/core/backlog.md` by reframing the two "Superseded by"
  Results blocks (former tasks #1 and #5) to reference the landed
  design doc (`docs/survey-pivot-design.md`) and the current code
  location (`src/multi_plan.rs`) rather than the about-to-disappear
  plan directory. Retired the stale `stack.yaml` exclusion bullet in
  the structured-data research task — the infrastructure no longer
  exists, so the exclusion is meaningless. Rewrote the "Migrate Ravel
  orchestrator" task's dependency from `survey-restructure/5d (✓ done)`
  to a direct commit-sha reference (`06ce874`), since the plan that
  owned 5d is going away but the commit history is permanent. Replaced
  the "deferred during survey-restructure wrap-up" framing on the
  task-count extraction task with "deferred during the 2026-04-21
  survey-pivot rescoping session" — same information, no live-plan
  dependency.
- **Step 3 (archive/retire):** removed the sibling entry for this plan
  from `LLM_STATE/core/related-plans.md`, leaving `## Siblings` with
  an explicit "_No active sibling plans._" placeholder. The plan
  directory `LLM_STATE/survey-restructure/` is intentionally left in
  place for the remainder of this cycle — analyse-work / reflect /
  triage still need to run against it, and `ravel-lite state
  set-phase` at the end of this work phase requires the directory
  to exist. After this cycle exits cleanly, the directory can be
  moved to `LLM_STATE/archive/survey-restructure/` or deleted; this
  is a one-step manual action the user can run at any point.

Verified with `rg survey-restructure LLM_STATE/core` returning no
matches. References remain in `docs/survey-pivot-design.md` (the
architectural design doc — historical, kept intentionally) and inside
the plan directory itself (to be archived along with it).

**Follow-ups surfaced but not scheduled here:**

- Decide concrete archive destination for `LLM_STATE/survey-restructure/`.
  A conventional `LLM_STATE/archive/` sibling directory would keep
  session-logs discoverable without polluting active-plan routing; a
  one-off manual `mv` after this cycle is sufficient.
- The "Migrate Ravel orchestrator off removed `push-plan` verb" task
  in `core/backlog.md` is now actionable (no blocking dependencies).
