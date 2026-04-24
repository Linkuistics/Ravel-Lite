You are running the DREAM phase of a multi-session backlog plan. The
dream phase runs periodically when memory has grown past the compaction
headroom. Its job is to rewrite memory losslessly in tighter form —
consolidating memories, like dreaming.

## Required reads

1. Current memory — run `ravel-lite state memory list {{PLAN}}`.
2. `{{ORCHESTRATOR}}/fixed-memory/memory-style.md` — the Memory style
   rules. Read this file directly.

## Do NOT read

Anything else. Not the backlog, not the session log, not the
latest-session record, not declared peer-project relationships. Fresh
context for rewriting means no task momentum, no session narrative —
just the text and the style rules.

## Behavior

1. Rewrite memory **in place, per entry**, applying the Memory style
   rules from `fixed-memory/memory-style.md`. For each entry that needs
   prose tightening:

   - Rewrite the body via
     `ravel-lite state memory set-body {{PLAN}} <id> --body-file <path>`
     (or `--body -` from stdin).
   - Rename the heading if the new phrasing is sharper, via
     `ravel-lite state memory set-title {{PLAN}} <id> "<new title>"`.

   Consolidate overlapping entries by rewriting one and deleting the
   other(s) via `ravel-lite state memory delete {{PLAN}} <id>`.

2. Your contract is **strictly lossless**. Preserve every live fact.
   Only rewrite prose. Do not delete entries unless they are pure
   duplicates (same claim, different wording). Reflect is the only
   lossy-pruning phase; dream is not.

3. Run `ravel-lite state set-phase {{PLAN}} git-commit-dream`.

4. Stop.

If the dream produces a bad result, it is recoverable:
`git checkout memory.yaml` restores the prior version. Memory is always
tracked in git.

## Output format

After completing the rewrite, print a brief summary using this structure.
Each entry is **two lines**: the label line carries the pre-change state;
a continuation line beginning with `→` carries the post-change state.

```
[OVERLAPPING] <heading A> + <heading B>
           → <result heading>
[VERBOSE] <heading> — <what was wordy>
       → <how it's now tightened>
[AWKWARD] <heading> — <old phrasing>
       → <new phrasing>
[STATS] <before word count>
     → <after word count>
```

Labels name the **state that caused the change**, not the action taken
(e.g. VERBOSE, not TIGHTENED). Two lines per significant change — old
on top, new under it. Minor prose edits can be omitted. End with the
STATS entry.

You may precede the action list with a brief reasoning preamble — what
patterns you noticed across the memory, what consolidations you
considered. Separate the preamble from the action list with a blank
line. Do not introduce other sections.
