# Plan status survey

You are producing a multi-project plan status overview for a developer
who wants to know which plan to run through `raveloop-cli run` next.
The plans follow the Raveloop convention: a directory is a plan iff it
contains `phase.md`; siblings `backlog.md` and `memory.md` hold task
state and distilled learnings.

Below (after the horizontal rule) are all discovered plans, grouped by
project. For each plan you have:

- project (root directory basename)
- plan (plan directory basename)
- phase (contents of `phase.md`)
- backlog (contents of `backlog.md`, or `(missing)` if absent)
- memory (contents of `memory.md`, or `(missing)` if absent)

Produce Markdown output with these sections in this exact order:

## Per-plan summary

One line per plan, sorted by project then plan name:

```
- `<project>/<plan>` — phase `<phase>`, <N> unblocked / <M> blocked / <K> done
```

If the plan's `backlog.md` contains a `## Received` heading with one
or more items under it (dispatches from other plans awaiting triage),
append ` ⚠ <N> unprocessed Received item(s)` to that plan's line.

If `backlog.md` or `memory.md` is missing, note it on the line rather
than guessing.

## Cross-project blockers

Any plan whose blockers reference work in a **different** project.
Format:

```
- `<project>/<plan>` blocked on `<other-project>/<other-plan>`: <one sentence>
```

If no cross-project blockers are detected, write `None detected.` and
move on.

## Recommended invocation order

Up to five plans to run through `raveloop-cli run` next, in priority
order. For each:

```
1. `<project>/<plan>` — <one sentence of rationale grounded in the files>
```

Prefer in this order:
1. Plans with unprocessed `## Received` items whose triage unblocks
   other plans on the critical path.
2. Plans with `not_started` tasks marked `P1` and no dependencies.
3. Independent research or literature-survey plans (cheap to run,
   often unblocked).

Skip plans whose only remaining work is `done` or `blocked` on
external input.

## Rules

- Do not speculate beyond what the files say.
- Do not recommend tasks that are already marked `done`.
- When a file is missing, note it; do not infer its contents.
- Keep each rationale to one sentence.
