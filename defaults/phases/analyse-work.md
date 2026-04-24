You are running the ANALYSE-WORK phase of a multi-session backlog plan.
This phase runs headlessly immediately after the interactive work phase
exits. Its job is to examine the actual changes made during the work
session, **commit every source-file change on behalf of the session**,
and produce a session log entry plus a git commit message for the
plan-state files.

You are analysing what happened from the ground truth (the diff), not
from an LLM's self-report. The diff is the authoritative record of what
changed.

## Work-tree snapshot

The orchestrator captured this snapshot after the work phase exited —
treat it as authoritative. Do **not** trust your own mental model of
what changed; use this block as the definitive list of paths to commit
or justify.

```
{{WORK_TREE_STATUS}}
```

## Backlog transitions since baseline

The orchestrator computed this diff between `backlog.yaml` at the work
baseline and `backlog.yaml` right now. Use it to understand which
backlog entries moved during the session — what was completed, added,
deleted, or reprioritised. This is authoritative — do **not** re-derive
it from the full diff.

```
{{BACKLOG_TRANSITIONS}}
```

Caveat: the baseline is the reflect commit of the previous cycle, so
task additions/deletions from the previous cycle's triage may also
appear in this block. Treat entries outside the current session's focus
as baseline context.

**Do not author commit titles or bodies around these task ids.** Commit
messages describe the work that happened in the tree, not the backlog
bookkeeping that framed it — see step 6 and step 9 for the full rule.

## Required reads

1. The task backlog — run `ravel-lite state backlog list {{PLAN}}`. You
   need to understand what task was being worked on and its recorded
   results.

## Do NOT read

- The memory (not needed for summarisation)
- Declared peer-project relationships (not relevant here)

## Behavior

1. Read `{{PLAN}}/work-baseline` to get the baseline commit SHA.
   `work-baseline` is a plain text file, not a state-CLI-managed one.

2. Run `git diff <baseline-sha> --stat` for an overview of what files
   changed.

3. Run `git diff <baseline-sha>` for the full diff. If the diff is very
   large, focus on the most significant changes rather than trying to
   process everything.

4. Inspect the backlog (from step 1's required read) to understand what
   task was being worked on and what results were recorded.

5. **Safety-net: repair stale task statuses.** Run
   `ravel-lite state backlog repair-stale-statuses {{PLAN}}`. The verb
   flips `in_progress` tasks with non-empty results to `done` and
   unblocks `blocked` tasks whose structural dependencies are all now
   `done`. It emits a YAML report of any repairs applied — if a repair
   occurred and it materially changed the backlog shape, note the
   *kind* of repair in the plan-state commit body (step 9) without
   naming specific task ids. If no drift is present this step is a
   no-op.

6. **Commit the source-file changes.** The work phase no longer commits
   its own source edits — that is this phase's responsibility. Using the
   work-tree snapshot above as ground truth:

   - Stage every path outside `{{PLAN}}/` that appears in the snapshot
     with `git add <path>` (or `git add -A -- :!{{PLAN}}` if the set is
     large — but explicit paths are preferred).
   - Commit with a descriptive message in the imperative mood that
     summarises the session's code/docs/config changes. The message
     describes what changed in the tree — the function added, the
     behaviour fixed, the file renamed. **Do not reference backlog
     task ids, session numbers, or phase names** (no
     `fix-work-baseline`, no `session 47`, no
     `during analyse-work`). Those belong to the plan state, not the
     source history. This is the commit that must make sense to anyone
     reading `git log` a year from now; it stands on its own without
     the plan-execution context. The plan-bookkeeping commit happens
     separately from `commit-message.md` (step 9).
   - If any path in the snapshot is intentionally **not** committed
     (e.g. a user scratch file, an accidental edit that should be
     reverted), you MUST name each such path explicitly in the session
     record (next step) with a one-sentence justification. Unjustified
     uncommitted paths will trigger a TUI warning.

   Do not commit files inside `{{PLAN}}/` here — those are reserved for
   the subsequent `git-commit-work` script phase.

7. Determine the session number by counting records returned from
   `ravel-lite state session-log list {{PLAN}}`, then add one. Record
   IDs themselves are free-form; sequential numbering in the entry body
   is the convention.

8. Write the session record via
   `ravel-lite state session-log set-latest {{PLAN}} --id <session-id> --timestamp <ts> --phase work --body-file <path>`
   where `<session-id>` is a short slug like `session-N`,
   `<ts>` is ISO 8601 UTC with seconds precision
   (`date -u '+%Y-%m-%dT%H:%M:%SZ'`), and `<path>` is a temp file whose
   content follows this layout:

   ```
   ### Session N (YYYY-MM-DDTHH:MM:SSZ) — brief title
   - What was attempted
   - What worked, what didn't
   - What this suggests trying next
   - Key learnings or discoveries

   ## Hand-offs (omit this section entirely when there are none)

   ### <hand-off title>
   - Problem being solved
   - Settled design decisions, each with a one-sentence rationale
   - Reference examples (file paths, line numbers)
   - Dependencies
   ```

   Base the entry on the actual diff and the backlog results, not
   assumptions. Be specific about what files were changed and why.

   **Hand-off convention.** A hand-off is a forward-looking design
   that this session settled but did NOT implement — a Q&A that
   resolved an approach, a reference example identified for a later
   task, a rejected alternative worth recording. Hand-offs must
   survive analyse-work → triage even when the implementing task is
   deleted next cycle. Record each hand-off by whichever channel fits:

   - **Preferred: promote directly to a new backlog task.** When the
     design is concrete enough to be picked up by a future work
     cycle, run
     `ravel-lite state backlog add {{PLAN}} --title "<title>" --category <cat> --description-file <path>`
     with `Status: not_started` (the default) and a description that
     **inlines** the settled design — not a one-liner pointer.
     Include: the problem being solved, each decision with a
     one-sentence rationale, reference examples (file paths, line
     numbers), and dependencies. Target 10–40 lines; enough that
     triage can promote without rereading the whole diff.

   - **Fallback: record in the `## Hand-offs` section above AND write
     a `[HANDOFF] <title>` hand-off block on the completing task
     via `ravel-lite state backlog set-handoff {{PLAN}} <task-id> --body-file <path>`.**
     Use this when the design is only partially settled. Triage mines
     hand-off blocks from completed tasks before deleting them (see
     `defaults/phases/triage.md` step 3).

9. Write `{{PLAN}}/commit-message.md` with a git commit message for the
   **plan-state commit** (the one `git-commit-work` will make next).
   `commit-message.md` is a one-shot scratch file, not a
   state-CLI-managed one, so write it directly. This is distinct from
   the source-file commit you made in step 6 and narrates the plan
   bookkeeping itself — the *kinds* of mutations that happened in
   `backlog.yaml`, `memory.yaml`, and the session log. Keep it tight:

   ```
   <title — imperative mood, max 72 chars, describes the kind of bookkeeping>

   <body — what kinds of bookkeeping and why, 2-5 lines>
   ```

   **Describe the change by shape, not by slot.** Use the
   `Backlog transitions since baseline` block above to understand what
   moved, then summarise in generic terms — the *kind* of change, not
   the identities it applied to. Good titles:

   - `Record results and flip status for the completed task`
   - `Add two new backlog tasks and update one description`
   - `Promote a hand-off to the backlog`
   - `Update backlog and memory` (fine as a catch-all for mixed sessions)

   Bad titles — do NOT use these patterns:

   - `Mark fix-work-baseline done, record results` — names a task id
   - `Add add-backlog-repair-stale-statuses to backlog` — names a task id
   - `run-plan: triage (core)` — names a phase and plan name
   - `Session 47 bookkeeping` — names a session number

   The body expands on the kinds of bookkeeping that happened and why,
   in one or two sentences. It does not name specific task ids, session
   numbers, phase names, or plan slugs. If the only way to describe the
   change is by task id, the commit scope is wrong — split the underlying
   work along semantic lines first.

   The title should still be specific enough to be useful in
   `git log --oneline` — it describes the *shape* of the bookkeeping
   (new tasks added, statuses flipped, a hand-off promoted), just not
   the identities involved.

10. Run `ravel-lite state set-phase {{PLAN}} git-commit-work`.

11. Stop.

## Output format

After completing all writes, print nothing. The driver displays the
commit message.
