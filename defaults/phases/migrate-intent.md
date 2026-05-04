You are running the `migrate-intent` one-shot phase of the
`ravel-lite migrate-v1-v2` verb.

# Inputs

- The v1 plan's `phase.md` (free-text intent prose) is at
  `{{NEW_PLAN_DIR}}/phase.md`.
- The v1 plan's `backlog.yaml` is at `{{NEW_PLAN_DIR}}/backlog.yaml`.
- The source repo's component catalog is available via:
  - `ravel-lite atlas list-components --repo {{SOURCE_REPO_SLUG}}`
  - `ravel-lite atlas describe <component-ref>` for any specific component
- The new plan name is `{{NEW_PLAN_NAME}}`.

# Task

Read `phase.md` and the backlog. Distil the plan's strategic intents
(typically 1–5) into TMS-shaped intent items, each with at least one
justification (rationale linking to the user's stated reasons in
`phase.md`).

For every backlog item, decide whether it serves one of your new
intents. If yes, attribute it. If no — it is genuinely unattributable
to any intent — mark it `legacy`.

# Output

Write `{{NEW_PLAN_DIR}}/migrate-intent-proposal.yaml` with this shape:

```yaml
intents:
  - id: i-001
    kind: intent
    claim: "<one-sentence strategic claim>"
    justifications:
      - kind: rationale
        text: "<why this intent exists, linking to phase.md prose>"
    status: active
    supersedes: []
    authored_at: "<ISO timestamp>"
    authored_in: "migrate-intent"
item_attributions:
  - item_id: <existing-backlog-item-id>
    serves: i-001            # or the literal "legacy" if unattributable
  - item_id: <other-id>
    serves: legacy
```

Then exit. Do not commit, do not edit any other file. The runner
applies your proposal mechanically.
