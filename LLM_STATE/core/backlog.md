# Backlog

## Tasks

### Integration test for `[HANDOFF]` convention in analyse-work → triage cycle

**Category:** `test`
**Status:** `done`
**Dependencies:** none — convention is live in both `defaults/phases/analyse-work.md` and `defaults/phases/triage.md`

**Description:**

Extend `ContractMockAgent` to inject `[HANDOFF]` markers into a Results
block and run a synthetic analyse-work → triage cycle. Assert that triage
correctly mines the marker and either promotes it to a new backlog task
or archives it to `memory.md`.

This was deferred from the "Preserve hand-off rationale" task. The
convention is now live in shipped prompts; the next real hand-off session
is the first end-to-end exercise, but an automated test guards the
pipeline before that.

**Deliverables:**

1. A `ContractMockAgent::invoke_headless` injection for `AnalyseWork`
   that emits a `[HANDOFF]` marker in a completing task's `Results:`
   block inside `latest-session.md`.
2. A test that runs a full analyse-work → git-commit-work cycle,
   then a triage cycle, and asserts the hand-off survives as either
   a new `not_started` backlog task or a new `memory.md` entry.

**Results:**

Implemented as two integration tests in `tests/integration.rs`:
`handoff_marker_in_analyse_work_is_promoted_by_triage` and
`handoff_marker_in_analyse_work_is_archived_by_triage`. Both run a full
analyse-work → git-commit-work → reflect → git-commit-reflect → triage
→ git-commit-triage cycle through `phase_loop`, then assert on the
end-of-cycle state of `backlog.md`, `memory.md`, and `latest-session.md`.

**What changed:**

- `ContractMockAgent` gained an opt-in `handoff_injection:
  Option<HandoffInjection>` field plus a `with_handoff_injection()`
  builder. When `None` (every existing test), behaviour is unchanged.
- When set, the `AnalyseWork` arm runs after the safety-net flip and
  (a) appends a `[HANDOFF] <title>\n<body>` block to the target task's
  `Results:` in `backlog.md`, (b) rewrites `latest-session.md` with a
  `## Hand-offs` section mirroring the marker. Ordering matters:
  safety-net must flip `Status: not_started → done` before the mining
  logic looks for `done` tasks.
- The `Triage` arm gained a parallel branch: when `handoff_injection`
  is set, scan every block for `Status: done`, extract any `[HANDOFF]`
  via `extract_handoff_from_block()`, either append a new
  `Status: not_started` task (`Promote`) or add a `## <title>` entry
  to `memory.md` (`Archive`), then drop the done task. The existing
  placeholder-append behaviour is preserved for tests that don't set
  an injection.
- Two new helpers live at module scope beside `flip_stale_task_statuses`:
  `inject_handoff_into_task_block` (analyse-work side) and
  `extract_handoff_from_block` (triage side). Both split on the
  existing `\n---` block separator convention used by the safety-net.
- One pre-existing `ContractMockAgent` struct-literal call site
  (`analyse_work_receives_snapshot_and_commits_uncommitted_source`)
  gained an explicit `handoff_injection: None` field.

**Verification:**

- `cargo test --test integration handoff_marker` → both new tests pass.
- Full integration suite: 25/25 pass; no regression in
  `analyse_work_receives_snapshot_and_commits_uncommitted_source`,
  `phase_contract_round_trip_writes_expected_files`, or the safety-net
  test, all of which share the `ContractMockAgent`.
- Full unit suite: 220/220 pass.
- `cargo clippy --all-targets -- -D warnings` — clean (exit 0).
  Fixed two pre-existing lints along the way at user request:
  six `doc_lazy_continuation` violations in `src/survey/schema.rs`
  (resolved by splitting the `input_hash` doc into paragraphs
  separated by blank `///` lines) and one `useless_format` in
  `tests/integration.rs:352` (replaced `format!(...)` with a literal
  `.to_string()`). Both were on `main` before this task; confirmed
  via `git stash` + rerun.

**What this suggests next:**

- The two tests are green against the current prompts, so the
  convention is now protected by CI. The next real hand-off session
  remains the first end-to-end exercise; if that session surfaces a
  shape the tests don't cover (multi-block hand-offs, nested code
  blocks in `handoff_body`), widen the helpers then.
- Clippy is now clean under `-D warnings`; consider adding a CI gate
  to keep it that way. The pre-existing `doc_lazy_continuation` and
  `useless_format` drift both escaped because no test step asserts
  clippy cleanliness — worth a future maintenance task.

---

### Research: expose plan-state markdown as structured data via `ravel-lite state <file> <verb>` CLI

**Category:** `research`
**Status:** `not_started`
**Dependencies:** `ravel-lite state` subcommand (✓ done — establishes the `state` namespace and the "CLI verb replaces direct file edit" pattern)

**Description:**

Investigate whether extending the `ravel-lite state` namespace with
structured read/write verbs over the plan's markdown surfaces —
`backlog.md` first, candidates also `memory.md`, `session-log.md`,
`related-plans.md`, `subagent-dispatch.yaml` — would meaningfully reduce
LLM context / tool-call cost and improve data-discipline, or whether
the benefits don't justify the added schema surface. Deliverable is a
design decision (go / no-go, with scope), not an implementation.

**Precedent the idea builds on.** Two verbs already convert free-form
file edits into typed CLI calls: `ravel-lite state set-phase` (replaces
`Write "reflect" to phase.md`) and `ravel-lite state push-plan`
(replaces the prose case-analysis for `stack.yaml`). Both landed
because (a) the target had a small well-defined schema, and (b)
enforcing invariants in Rust was tractable and the invariants were
load-bearing. The question this task asks is whether that pattern
scales up to larger, looser surfaces like `backlog.md`.

**Concrete shape of the proposal being evaluated:**

```
ravel-lite state backlog list [--status <s>] [--category <c>] [--ready] [--format json|table]
ravel-lite state backlog show <id>
ravel-lite state backlog add --title <t> --category <c> [--dependencies <d,d>] [--description-file <path>]
ravel-lite state backlog set-status <id> <status>
ravel-lite state backlog set-results <id> <results-file>
ravel-lite state backlog delete <id>
ravel-lite state backlog reorder <id> <before|after> <target-id>

ravel-lite state memory list [--format json]
ravel-lite state memory add --title <t> --body-file <path>
ravel-lite state memory delete <id>

ravel-lite state session-log append --session-file <path>

ravel-lite state related-plans list [--kind parent|child]
```

`<id>` could be the task title (slugified) or an ordinal; the research
should settle which.

**Potential benefits (hypotheses to test, not established):**

1. **Context reduction.** An LLM in the work phase currently reads
   all of `backlog.md` to pick a task. Today's file is ~450 lines;
   `backlog list --status not_started` would return maybe 20 lines.
   Measurable win if the savings are consistent across plan sizes.
2. **Tool-call reduction.** Current mutate pattern is Read + Edit
   (Edit requires exact-string match, often preceded by a probe Read
   to find the anchor). `set-status <id> done` collapses both sides.
   Analyse-work's safety-net step — find tasks with non-empty Results
   and stale Status and flip each — becomes a single command.
3. **Schema enforcement.** Writing invalid status values (`"pending"`
   when the vocabulary is `not_started / in_progress / done /
   blocked`) becomes a parse error, not silent drift. Catches
   mistakes the current prompts' prose guidance does not.
4. **Atomic mutations.** No TOCTOU window between Read and Write.
   Multiple prompts co-editing the same file in rapid succession
   (rare today, possible with parallel subagents tomorrow) stop
   racing.
5. **Typed queries unlock tooling.** `ready` = "status=not_started
   AND dependencies are all done" is currently a prose rule the LLM
   applies by reading. A CLI verb could expose it as a query —
   triage and work both win.

**Tradeoffs / risks to evaluate:**

1. **"All state is a readable file" principle (README §Principles).**
   Today a user can open `backlog.md` in their editor and edit
   anything. A CLI-emitted file is still readable, but hand-edits
   that break the schema (even minor: a missing blank line between
   fields) become errors on the next CLI read. The fix is a
   permissive parser that canonicalises on write — but that's its
   own drift surface.
2. **Free-form description authorship is awkward as CLI args.**
   Task descriptions are multi-paragraph markdown with code blocks,
   headings, and tables. `--description-file <path>` works but
   reintroduces the file-edit loop. Evaluate whether the LLM's
   actual authoring patterns fit a CLI.
3. **Schema migration cost.** The backlog's current schema has
   grown organically (Category / Status / Dependencies / Description
   / Results). Formalising it freezes today's shape. Adding a field
   later requires either schema versioning or a parser permissive
   enough to ignore unknown fields.
4. **Parser complexity.** Markdown is easy to emit, hard to parse
   consistently. Either adopt an explicit structured sidecar
   (backlog.yaml alongside backlog.md, regenerated on write), or
   constrain the markdown to a strict subset and write a dedicated
   parser. Sidecar loses the "single readable file" property;
   dedicated parser is a maintenance burden.
5. **Partial adoption creates drift.** If only some phase prompts
   use the CLI and others keep writing markdown directly, the two
   paths must stay consistent. Requires either an all-at-once prompt
   migration or a long coexistence period with both paths tested.
6. **Opacity for the LLM in reasoning tasks.** `list --status open`
   is great for selection. But triage explicitly *reasons over* the
   full backlog to detect buried blockers (triage.md:46-50). A
   structured list would need to preserve enough narrative per task
   (or stream full descriptions on demand) for that reasoning to
   still work — else triage quality regresses.

**Research questions the design must answer:**

- **Q1 — Authoritative format.** Markdown-as-source-of-truth (CLI
  parses + rewrites) vs structured sidecar (markdown is a rendered
  view) vs canonical markdown with a strict grammar. Recommendation
  with justification required.
- **Q2 — Which files qualify?** Backlog is the strongest candidate.
  Memory is semi-structured (`##` heading + prose body). Session-log
  is append-only. Related-plans is a categorised path list.
  Rank by benefit/cost and propose an incremental rollout order.
- **Q3 — Scope of the `list` query DSL.** Must cover: open,
  by status, by category, by dependency-readiness, by
  missing-results (analyse-work's safety-net), by age. Bikeshed-prone
  — settle the minimum useful set.
- **Q4 — Output formats.** `--format table` for humans, `--format
  json` for LLMs? Or markdown? Prompts currently consume markdown
  natively; JSON changes the reasoning surface.
- **Q5 — Identity.** Slug from title, stable ordinal, UUID? Titles
  change; ordinals shift on delete; UUIDs are LLM-unfriendly. Trade-off.
- **Q6 — Results-block authorship.** The most-edited piece of a
  backlog task is the Results block, which is often a 20-100 line
  markdown document with code blocks and insight. The CLI's story
  for this has to be clear: does the LLM write a file and invoke
  `set-results <id> <file>`, stream on stdin, or stay on Read+Edit
  for this field only?
- **Q7 — User hand-edit compatibility.** How permissive is the
  parser on read? What happens if a user adds a new field like
  `**Priority:** high`? Preserve-and-pass-through, error, or
  silently drop?
- **Q8 — Migration path.** If the answer is "go," how do existing
  plans migrate? One-shot reformat command? Gradual (CLI writes
  canonical, reads permissive, files converge over time)?

**Evaluation criteria (for deciding go / partial / no-go):**

- **Context savings** estimated per phase (work, analyse-work,
  triage) — rough token-count delta for a representative plan.
- **Tool-call delta** per phase — how many Read/Edit calls removed.
- **Invariant coverage** — what classes of silent drift (invalid
  status, missing Results, dangling dependencies) become enforced
  errors.
- **Implementation cost** — rough LOC estimate for parser +
  emitter + CLI verbs for the recommended scope.
- **Prompt-update cost** — how many of the 5 shipped phase prompts
  and `create-plan.md` need revision.
- **Principle cost** — does the preferred design still satisfy
  "All config, prompts, phase state, and memory are readable files
  on disk" from README §Principles? If not, by how much?

**Deliverables (of this research task):**

1. A design doc (markdown, committed to `docs/` or in-session) that
   answers Q1-Q8 with explicit decisions and rationale.
2. Recommended rollout: either one or more concrete follow-on
   backlog tasks (sized for individual work phases), or a
   documented "no-go" with justification.
3. If the recommendation is go: a prototype proof-of-concept for
   `backlog list --status not_started --format json` that parses
   the current `LLM_STATE/core/backlog.md` without data loss, to
   validate the parser-feasibility assumption before committing to
   a full rollout.

**Out of scope of this research task:**

- Implementation of the full CLI. This task is research +
  prototype only.
- Changes to any phase prompt. Prompt migration is a downstream
  task that only happens if the research concludes go.
- Changes to agent-config files (`config.yaml`, `tokens.yaml`) or
  the `survey.md` / `create-plan.md` prompts. Those aren't
  plan-state and aren't part of the hypothesis.
- Any stack-coordinator infrastructure — `stack.yaml`, `push-plan`,
  `pivot.rs`, `run_stack` have all been removed from the codebase;
  no longer a consideration.
- Any file under `fixed-memory/` — those are static documentation,
  not plan state.

**Related context:**

- Memory entry `Phase prompts invoke 'ravel-lite state set-phase'`
  records the convention this task generalises.
- The "Preserve hand-off rationale" task (now done) means
  Q6 can rely on the `[HANDOFF]` convention in Results blocks.
  The research question is narrower as a result: the Results block
  authorship path only needs to support the now-stable convention.
- Once this task settles, it unblocks "Move per-plan task-count extraction
  from LLM survey prompt into Rust" (see task below).

**Results:** _pending_

---

### Move per-plan task-count extraction from LLM survey prompt into Rust

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** "Research: expose plan-state markdown as structured data" (task above) — requires a structured backlog parser before task counts can be derived in Rust

**Description:**

The survey LLM currently infers per-plan task counts from the raw markdown in
`backlog.md`. Once the structured backlog parser from the research task above
exists, task counts (total, not_started, in_progress, done) can be computed
directly in Rust and injected as pre-populated tokens into the survey prompt —
removing an unnecessary inference burden from the LLM.

Identified as a deferred follow-on during the 2026-04-21 survey-pivot
rescoping session. Do not schedule until the structured-data research
task resolves; that task's completion is the trigger to revisit scope
here.

**Deliverables:**

1. Extend the structured backlog parser to expose a `task_counts() -> TaskCounts`
   method.
2. In `src/survey/discover.rs`, compute task counts from the parsed backlog
   and inject them into `PlanRow` (replacing the LLM-inferred field).
3. Update `defaults/survey.md` to remove the instruction asking the LLM
   to count tasks; add a note that counts are pre-populated.
4. Test: assert counts are correct for a plan with tasks in each status.

**Results:** _pending_

---

### Remove Claude Code `--debug-file` workaround once version exceeds 2.1.116

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`invoke_interactive` in `src/agent/claude_code.rs` passes
`--debug-file /tmp/claude-debug.log` as a workaround for a TUI
rendering failure in Claude Code ≤2.1.116. The root cause was not
found; debug mode happens to mask it via an unknown upstream mechanism.

When the installed `claude` binary is updated past 2.1.116, remove both
`args.push` lines adding `--debug-file` and `/tmp/claude-debug.log`.
Verify that the Work phase TUI renders correctly without the flag
before closing.

**Results:** _pending_

---
