You are running the TRIAGE phase of a multi-session backlog plan. The
triage phase runs headlessly at the end of each cycle. Its job is to
review and adjust the task backlog based on what the cycle learned, and
to emit a structured list of cross-plan propagations for the driver to
dispatch after this phase exits.

## Required reads

1. `{{PLAN}}/backlog.md` — the task backlog
2. `{{PLAN}}/memory.md` — distilled learnings

## Related plans

{{RELATED_PLANS}}

## Do NOT read

- `{{PLAN}}/session-log.md`
- `{{PLAN}}/latest-session.md`
- **Any file under a sibling, parent, or child plan directory.**
  Cross-plan awareness comes from the Related plans block above (paths
  only). You do not read foreign plan content from this phase — the
  driver dispatches fresh pi processes per propagation target after
  you exit, and each of those processes reads its own target's
  backlog/memory with fresh context.

## Behavior

### 1. Local triage

Review each task in `backlog.md`:

- Still relevant?
- Priority changed?
- Needs splitting?

Add new tasks implied by learnings in `memory.md`. **Delete completed
tasks.** Remove any task with status `done`, and clear any "Completed
Tasks" section entirely — heading and all. Reflect has already run and
anything worth keeping is now in `memory.md`; the session-log entry is
the durable record of what happened. The backlog is for work that
still needs doing, and must never carry a standing "Completed" holding
area between cycles.

Remove tasks that are no longer relevant (dependencies met, approach
changed, out of scope). Reprioritize based on what the cycle revealed.

**Scan task descriptions for embedded blockers.** A spike, validation
step, or shared dependency buried inside one task's description is
invisible to future work phases until that task runs — even when it
could run in parallel today. Promote any such blocker to its own
top-level task so it surfaces as executable work.

### 2. Cross-plan propagation — emit a structured list

Look at the Related plans block above. For each listed plan (siblings,
parents, children), judge whether this session's learnings affect that
plan. Rules of thumb:

- **Children:** if the learning changes how downstream consumers should
  use this plan's outputs, it affects children.
- **Parents:** if the learning reveals a bug, limitation, or gap in
  something parents produce, it affects parents.
- **Siblings:** if the learning generalizes to a shared pattern across
  siblings, it affects siblings.

**Write** `{{PLAN}}/propagation.out.yaml` containing one entry per
related plan that warrants propagation. Use this exact format:

```yaml
propagations:
  - target: /absolute/path/to/related/plan
    kind: child         # or "parent" or "sibling"
    summary: |
      One to three paragraphs describing the learning and why it
      affects this target. The receiving pi process will be given
      this summary plus the target path, and told to read the
      target's backlog.md and memory.md and apply whatever updates
      are warranted.
```

Rules:
- Use absolute paths (the Related plans block already shows them).
- Use `|` (block scalar) for `summary` so multi-line text works.
- Omit the whole file if there are no propagations. An empty or absent
  `propagation.out.yaml` tells the driver there is nothing to fan out.
- Do **not** attempt to dispatch anything yourself. You do not have a
  subagent tool and should not try to invoke one. The driver reads
  `propagation.out.yaml` after you exit and handles dispatch.

### 3. Finishing

Write `work` to `{{PLAN}}/phase.md`. Stop.
