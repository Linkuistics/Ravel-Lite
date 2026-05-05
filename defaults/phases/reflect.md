You are running the REFLECT phase of a multi-session backlog plan. The
reflect phase runs headlessly after the work phase exits. Its job is to
distill learnings from the latest session into durable memory, validate
existing memory against the current code, and check this cycle's work
against active intents for drift. The Memory style rules apply
throughout.

## Required reads

1. The current session's entry — run
   `ravel-lite state session-log show-latest {{PLAN}}`. This is the only
   session input reflect sees.
2. Current distilled memory — run
   `ravel-lite state memory list {{PLAN}}`.
3. Active intents — run `ravel-lite state intents list {{PLAN}}`. The
   intent-trajectory check (step 3 below) needs these. If the file is
   absent (legacy v1 plan), skip the trajectory check.
4. The Memory style rules — run
   `Bash(ravel-lite fixed-memory show memory-style)`.

## Do NOT read

- The task backlog (avoids task-oriented thinking during reflection)
- The session log history (append-only audit trail; never read by any
  LLM phase)
- Declared peer-project relationships (paths only; never read or
  mutate another plan's files — record cross-plan observations as
  findings per step 4 instead)

## Behavior

### 1. Bounded TMS check on existing memory

Before distilling new entries, validate existing ones against the
current code. Run

    ravel-lite state memory check-anchors {{PLAN}}

The verb walks every active memory entry's `code-anchor` justifications
and emits a YAML `SuspectReport` with two failure modes:

- `path-missing` — the file at `path` no longer exists.
- `sha-mismatch` — the file exists but its blob SHA differs from
  `sha_at_assertion`.

For each suspect, re-read the relevant code and decide:

- **Still true.** The drift is cosmetic (whitespace, surrounding
  refactor that did not invalidate the claim). Leave the entry alone;
  the SHA can be refreshed when the entry is next genuinely updated.
- **Refined.** The claim is partly right but needs sharpening — update
  the body via `state memory set-body` (and `set-title` if the
  assertion shifted).
- **Defeated.** The claim is no longer true. Run
  `ravel-lite state memory set-status {{PLAN}} <id> defeated`. If a
  successor entry replaces it, author the successor first and pass
  `--supersedes <old-id>` (when supported) or note the relationship in
  the new entry's body.

For entries with **rationale-only justifications** that the session
clearly affected, apply the same re-evaluation pass even though they
won't appear in the suspect report — code-anchor justifications cover
mechanically-grounded claims, not all of them.

### 2. Distil session learnings

For each learning in the latest session, decide against current memory:

- Is this new? → add a memory entry with
  `ravel-lite state memory add {{PLAN}} --title "<heading>" --body-file <path>`
  (or `--body -` piped from stdin).
- Does this sharpen an existing entry? → update the body with
  `ravel-lite state memory set-body {{PLAN}} <id> --body-file <path>`;
  rename the heading with
  `ravel-lite state memory set-title {{PLAN}} <id> "<new title>"` if
  the assertion changed.
- Does this contradict an existing entry? → defeat the old entry
  (`set-status … defeated`) and author the new one fresh. Two
  contradictory active entries is a bug; the TMS substrate has no way
  to reconcile them.
- Does this make an existing entry redundant? → delete with
  `ravel-lite state memory delete {{PLAN}} <id>`.

When writing new or updated memory entries, follow the Memory style
rules from `ravel-lite fixed-memory show memory-style` exactly: assertion register
(not narrative), one fact per entry, cross-reference over re-explanation,
short subject-predicate headings, no session numbers or dates.

**Authoring discipline.** When a claim is grounded in specific code,
attach one or more `code-anchor` justifications so the bounded check
(step 1) can mechanically detect drift in future cycles. Pass them via
the repeatable `--code-anchor` flag on `state memory add`:

```
ravel-lite state memory add {{PLAN}} \
  --title "..." --body-file <path> \
  --code-anchor "component=<ref>,path=<rel-path>,sha=<blob-sha>[,lines=<a>-<b>]"
```

`sha` is the git blob SHA of `path` at the moment of authoring — compute
it with `git hash-object <path>` before invoking the verb. `component`
is a `ComponentRef` (`<repo>:<component-path>`); `lines` is optional.
The flag is repeatable for entries grounded in multiple files.

Prune aggressively. Memory should contain only what is currently true
and useful, not a historical record. The session log is the safety
net for discarded content.

### 3. Intent-trajectory check

If `state intents list` returned active intents, walk this cycle's work
against them:

- For each active intent, was any session activity in service of it?
  (Code edits, memory entries, decisions captured in the session log.)
- For each substantial piece of session activity, does it serve any
  active intent?

Two failure modes worth surfacing:

- **Under-served intents.** Intents whose claim has had no movement for
  several cycles. The plan may have drifted past them, or they may
  need supersession.
- **Off-trajectory work.** Session activity that does not line up with
  any active intent. May be legitimate (an opportunistic fix) or may
  be drift worth naming.

If you find drift that's worth flagging, append a single memory entry
with title `Intent-trajectory: <one-line summary>` capturing the
specific intents involved and the specific drift observed. This is
*advisory output*: the user may have legitimate reasons for the drift,
but surfacing it prevents silent accumulation across many cycles. Do
not author one of these entries every cycle by default — only when
something concrete is worth recording.

### 4. Cross-plan findings

If distillation surfaces an observation that is out of scope for this
plan but worth attention elsewhere — a cross-cutting issue you noticed
in another component, a defect in code you read but did not edit, a
suggestion that belongs in a different plan — record it in the
context-level findings inbox:

    ravel-lite findings add \
      --claim "<one-line assertion>" \
      --body-file <path> \
      --raised-in {{PLAN}} \
      --authored-in reflect \
      [--component <repo>:<component>]

Findings are advisory and the user processes them out of band; reflect
must not mutate other plans. Skip this step entirely when there is
nothing to record. For each finding written, include a
`[FINDING] <one-line claim>` line in your narrative preamble.

### 5. Hand off

Run `ravel-lite state set-phase {{PLAN}} git-commit-reflect`. Reflect
closes the cycle: after the runner commits reflect's plan-state
edits and saves `triage-baseline`, the cycle ends and a fresh
`ravel-lite run` picks up at triage.

Stop.

## Output format

Your output has two parts, in order:

1. A narrative preamble — a brief paragraph on what you noticed in the
   session and what trade-offs drove your choices. **Inside this
   preamble, include one line per intent-bearing action** using:

   - `[IMPRECISE] <heading> — <what was vague, how it is now sharper>`
     for entries you sharpened rather than replaced. The renderer
     classifies any title-or-body change as `[STALE]`; `[IMPRECISE]`
     is the subtype that says the prior wording was technically
     correct but vague — only you can distinguish that from a true
     replacement.
   - `[SUSPECT] <heading> — <what the bounded check flagged, what you did>`
     when step 1 surfaced a memory entry whose code-anchor drifted.
     Decision must be one of `still-true | refined | defeated`. The
     renderer's structural diff catches the body/status mutation; this
     line records the *why*.
   - `[TRAJECTORY] <intent claim> — <observed drift>` if step 3 led you
     to author an intent-trajectory memory entry. Cite the specific
     intent ids the entry concerns.
   - `[FINDING] <one-line claim>` (step 4) when you wrote a cross-plan
     finding to the context-level inbox.

   These complement — they do not replace — the renderer's structural
   output below. The renderer reports the "what" (NEW / STALE /
   OBSOLETE per id-level diff); intent labels report the "why" the
   diff cannot recover.

2. A blank line, then the renderer's structural label list, produced by
   running:

       ravel-lite state phase-summary render {{PLAN}} --phase reflect \
           --baseline $(cat {{PLAN}}/reflect-baseline 2>/dev/null || echo "")

   Emit the renderer's output verbatim. Do not add, remove, or reorder
   its lines.

Do not introduce other sections.
