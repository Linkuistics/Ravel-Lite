You are running the `migrate-memory-backfill` one-shot phase of the
`ravel-lite migrate-v1-v2` verb.

# Inputs

- The plan's existing memory entries are at
  `{{NEW_PLAN_DIR}}/memory.yaml`. They are already TMS-shaped (claim,
  justifications, status). What's missing is `attribution`.
- The mounted targets are listed in `{{NEW_PLAN_DIR}}/targets.yaml`.
- The components in those targets are available via
  `ravel-lite atlas describe <component-ref>`.

# Task

For every memory entry, decide its `attribution`:

- A `<repo_slug>:<component_id>` ComponentRef when the lesson is
  about a specific component (typically one of the mounted targets).
- The literal `plan-process` when the lesson is about how the plan
  itself ran (work-style, dispatch ergonomics, etc.) and doesn't
  belong to any code component.
- Null (omit `attribution` or set to ~) when you genuinely can't
  attribute the entry. The runner will set `status: legacy` on these
  for the user to curate.

# Output

Write `{{NEW_PLAN_DIR}}/migrate-memory-proposal.yaml`:

```yaml
attributions:
  - entry_id: <existing-memory-entry-id>
    attribution: atlas:atlas-core
  - entry_id: <other-id>
    attribution: plan-process
  - entry_id: <unattributable-id>
    attribution: ~
```

Then exit. The runner mutates `memory.yaml` mechanically.
