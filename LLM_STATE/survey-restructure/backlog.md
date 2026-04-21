# Backlog

## Tasks

### Wrap-up — Merge survey-restructure branch, propagate completion, archive plan

**Category:** `housekeeping`
**Status:** `not_started`
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
