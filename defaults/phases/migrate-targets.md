You are running the `migrate-targets` one-shot phase of the
`ravel-lite migrate-v1-v2` verb.

# Inputs

- Source repo slug: `{{SOURCE_REPO_SLUG}}`.
- Source repo's components are available via:
  - `ravel-lite atlas list-components --repo {{SOURCE_REPO_SLUG}} --format yaml`
    — emits a structured envelope. Use the `id:` field of each record
    verbatim — it is already the canonical component id (a path of
    kebab-case segments, e.g. `ravel-lite/knowledge-graph` or
    `atlas/atlas-cli/mycli`).
  - `ravel-lite atlas describe {{SOURCE_REPO_SLUG}}:<component-id>` for details.
- The plan's intents have already been extracted to
  `{{NEW_PLAN_DIR}}/intents.yaml`.

# Task

Identify which components in `{{SOURCE_REPO_SLUG}}` this plan will need
to *edit* (not merely read). The heuristic: a component is a target if
at least one of the plan's active intents references it (by name in the
intent's claim or justifications) OR if at least one backlog item lists
code anchors inside it.

Cross-repo targets are not supported in v1 of this migrator — all
targets must live in `{{SOURCE_REPO_SLUG}}`.

# Output

Write `{{NEW_PLAN_DIR}}/migrate-targets-proposal.yaml`:

```yaml
targets:
  - component_id: <id-from-yaml-envelope>
  - component_id: <other-id>
```

`component_id` is the `id:` field from `atlas list-components --format
yaml`, copied verbatim. Do not strip, split, or otherwise modify it.

Then exit. The runner mounts a git worktree per target via existing
`mount_target` machinery.
