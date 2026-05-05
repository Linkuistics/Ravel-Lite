# Architecture (next)

> Status: design exploration. Captures the redesign worked through in
> conversation, covering multi-component/multi-repo plans, worktrees,
> a queryable knowledge graph backed by `.atlas/` data in each repo,
> a unified TMS substrate over plan state (intents, backlog, memory,
> findings), and component-attached memory in the codebase. Not yet a
> committed plan — see *Open questions* and *Migration* at the end.

## Why this redesign

Today's `<project>/LLM_STATE/<plan>` model couples a plan to a single
project subtree. The cross-project mechanism (subagent dispatch fanning
out into sibling plans) has poor UX once a plan's intent really spans
components, and "project" is the wrong granularity once a single git
repo can host many small components. Multi-repo systems — the common
case once Atlas is mapping a real component graph that crosses git
boundaries — get no first-class treatment at all.

The redesign:

- **Components, not projects, are the unit of plan-targeting.** A
  plan targets one or more *components* (Atlas-defined entities), each
  of which lives in one git root. A plan can cross git roots; a
  component cannot.
- **Plans live in a shared ravel context** with its own git identity,
  separate from any target repo. Cross-component plans become "one
  plan, several targets," not "several coordinating plans."
- **Worktrees isolate plan work per repo.** Each plan gets its own
  branch and worktree per touched repo; the plan's commits land on
  per-repo plan branches, merged to main via PR or fast-forward at
  plan finish.
- **The catalog is a queryable graph**, not a prompt-embedded YAML
  dump. The agent explores it on demand via `ravel-lite atlas <q>`.
- **Plan state is a unified TMS substrate.** Intents, backlog,
  memory, and findings share one schema with claims, justifications,
  status, and supersession. Reflect and curate maintain truth via the
  same engine; defeat cascades automatically.
- **Memory is structured (TMS)**, separated into transient plan
  memory and durable component memory committed alongside the code
  it describes.

## Principles preserved

- **No magic.** All config, prompts, plan state, memory, and catalog
  data are readable files on disk.
- **Visible, auditable, adjustable.** Every input is a file the user
  can inspect and edit; every state transition writes to the
  filesystem.
- **Agents are subprocesses.** The orchestrator never calls LLM APIs
  directly.
- **Mechanical work belongs in Rust.** The LLM declares intent (via
  scratch files like `commits.yaml`, `target-requests.yaml`,
  `focus-objections.yaml`, `promotion-spec.yaml`); the runner
  executes it. Counting, partitioning, file ops, branching, mounting,
  cascade — Rust.
- **Atlas owns its outputs.** Ravel-lite reads `.atlas/` data; never
  writes catalog files; never extends the schema beyond what
  atlas-contracts publishes.

## Vocabulary

- **Ravel context** — the user's single root, holding configuration,
  the repo registry, and all plans. One per user. Has its own git
  identity, separate from every target repo.
- **Component** — an Atlas-defined entity (`ComponentEntry` in
  atlas-contracts) with an id, kind, path segments, and a parent
  chain. The unit of plan-targeting.
- **ComponentRef** — `(repo_slug, component_id)`. Globally unique
  across the ravel context. Used by targets, edges, memory
  attribution, and `this-cycle-focus.yaml`.
- **Target** — a component projected into a plan: a ComponentRef plus
  the worktree root, branch, and path segments cached at mount time.
- **Plan** — a top-level unit of intent. Spans targets across
  potentially multiple git roots; has its own state, memory, and
  branches.
- **Intent** — a strategic claim about what the plan is for, expressed
  as a TMS item. Justifies backlog items via `serves-intent` edges.
  Plan completion is intent-shaped, not backlog-shaped.
- **TMS item / knowledge item** — a node in the plan's or the
  catalog's knowledge graph. Has a claim, justifications, status, and
  provenance. The four ravel-lite kinds are intents, backlog items,
  memory entries, findings.
- **Cycle** — one iteration of `triage → work → analyse-work →
  reflect`. Single-target by default.
- **Catalog** — the union of `.atlas/components.yaml` files across
  all registered repos, viewed as a graph.

## Layout

```
<ravel-context>/                           # one git identity, the user's ravel home
├── config.yaml
├── config.local.yaml
├── repos.yaml                             # registry of known repos
├── findings.yaml                          # context-level inbox (cross-cutting suggestions, TMS-shaped)
├── agents/                                # claude-code, pi configs (as today)
├── phases/                                # phase prompt templates (as today)
├── fixed-memory/                          # shared style guides (as today)
└── plans/
    ├── foo/                               # an active plan
    │   ├── phase.md                       # rendered plan overview (canonical: intents.yaml)
    │   ├── intents.yaml                   # strategic intents (TMS-shaped)
    │   ├── backlog.yaml                   # tactical items (TMS-shaped, justified by intents)
    │   ├── memory.yaml                    # transient memory (TMS-shaped)
    │   ├── targets.yaml                   # mounted targets (runtime state)
    │   ├── target-requests.yaml           # one-shot scratch (when present)
    │   ├── focus-objections.yaml          # one-shot, written by work phase
    │   ├── this-cycle-focus.yaml          # one-shot, written by triage
    │   ├── commits.yaml                   # one-shot, written by analyse-work
    │   ├── promotion-spec.yaml            # one-shot, written at plan finish
    │   ├── work-baseline.yaml             # per-worktree baseline SHAs
    │   └── .worktrees/                    # gitignored
    │       ├── atlas/                     # worktree of atlas repo on plan branch
    │       └── sidekick/
    └── bar/
```

In each registered repo:

```
<repo>/
├── .atlas/                                # Atlas-owned, committed to the repo
│   ├── components.yaml                    # ComponentsFile { root: <repo>, components: [...] }
│   ├── components.overrides.yaml
│   ├── external-components.yaml
│   └── related-components.yaml
└── <component-path>/.atlas/
    └── memory.yaml                        # component memory, TMS-structured
```

`.atlas/` is committed, not gitignored. Catalog data and component
memory travel with the code they describe.

## The repo registry

`<context>/repos.yaml` is human-curated and lists every repo the user
wants to be able to target:

```yaml
repos:
  atlas:
    url: git@github.com:antony/atlas.git
    local_path: /Users/antony/Development/atlas       # optional
  ravel-lite:
    url: git@github.com:antony/Ravel-Lite.git
    local_path: /Users/antony/Development/Ravel-Lite
  atlas-contracts:
    url: git@github.com:linkuistics/atlas-contracts.git
    local_path: /Users/antony/Development/atlas-contracts
```

`local_path` is the user's regular checkout. Ravel-lite reads
`.atlas/` from there for catalog views; mounts new worktrees from it
(fast, shares the object database). When `local_path` is absent,
ravel-lite clones into `<context>/.cache/repos/<name>/` on demand.

There's no aggregate catalog file. The catalog view is computed on
read as the union of `.atlas/components.yaml` from each registered
repo. ComponentRefs use the repos.yaml key as `repo_slug`.

## Knowledge substrate (TMS + KG)

The four state files in a plan — `intents.yaml`, `backlog.yaml`,
`memory.yaml`, and the context-level `findings.yaml` — share a single
underlying data structure. So does the catalog graph aggregated from
`.atlas/`. The substrate is a **typed knowledge graph with
defeasibility (TMS)**: nodes are claims with justifications and
status; edges are typed relationships; status changes propagate via
cascade.

This substrate is intended to be **extracted into a generally-
reusable crate**, consumed by ravel-lite for plan state and by Atlas
(eventually) for the catalog graph. The library is independent of
either application's vocabulary; domain types (intent, backlog item,
component, edge kind) are layered on top.

### Item shape

Every TMS item — across all kinds — has the same generic shape:

```yaml
id: <kind>-<timestamp>-<n>      # globally unique within scope
kind: intent | backlog-item | memory-entry | finding   # plus future kinds
claim: ...                       # the assertion or imperative
justifications:
  - kind: code-anchor | rationale | serves-intent | external | ...
    ... (kind-specific fields)
status: ...                      # see Status vocabularies
supersedes: [<id>, ...]          # this item replaces these (optional)
superseded_by: <id>              # this item was replaced by another
defeated_by: <id|cascade>        # this item was retracted
authored_at: <timestamp>
authored_in: <provenance>        # e.g., create | triage:c-7 | reflect:c-9
```

Kind-specific extensions live alongside without breaking the generic
shape: `attribution: <component-ref>` for memory entries headed for
promotion; `blocked_by: [<id>...]` for backlog items; etc.

### Justification vocabulary

Justifications are typed edges in the knowledge graph. The vocabulary,
though small, is load-bearing — it determines what the TMS engine can
reason about mechanically:

- `code-anchor` — `{ component, path, lines, sha_at_assertion }`.
  Mechanically verifiable: does the file exist, does the SHA still
  match.
- `rationale` — `{ text }`. Free prose; not mechanically verifiable;
  LLM-evaluated when needed.
- `serves-intent` — `{ intent_id }`. Backlog item justified by a
  parent intent. Cascade carrier.
- `defeats` / `supersedes` — `{ item_id }`. Linkage to other items.
- `external` — `{ uri }`. Links to issue trackers, design docs,
  conversations.

New justification kinds get added as the system grows.

### Status vocabularies

Each item kind has its own status enum, but the operations are
uniform:

- **Intent:** `active | satisfied | defeated | superseded`.
- **Backlog item:** `active | done | defeated | superseded | blocked`.
- **Memory entry:** `active | defeated | superseded`.
- **Finding:** `new | promoted | wontfix | superseded`.

Transitions are constrained (an active item cannot go directly to
`superseded` without a successor; a `done` backlog item is terminal;
etc.) but the constraint table is per-kind data, not per-kind code.

### Defeat cascade

Defeating an item with downstream dependents triggers a cascade:

- An intent's defeat propagates to backlog items justified by it via
  `serves-intent`. If an item has no other active justifying intent,
  it's also defeated, with `defeated_by: cascade`.
- A backlog item's defeat propagates to items that listed it as
  `blocked_by` — those become unblocked.
- A memory entry's defeat propagates to other entries that justified
  themselves by reference to it.

Cascade runs deterministically at any phase boundary that mutates
status. Implemented in Rust against the typed graph; not the LLM's
job.

### Plan as a knowledge graph

Within a single plan, the items across all four kinds form a small
connected graph:

```
                  ┌──────────────────────────┐
                  │ Intent i-001             │
                  │ "evidence-graded edges"  │
                  └──┬───────────────────┬───┘
                     │ serves-intent     │ serves-intent
              ┌──────▼───────┐    ┌──────▼────────┐
              │ Backlog t-001│    │ Backlog t-005 │
              └──────┬───────┘    └──────┬────────┘
                     │ derived-from           ↑ defeats
              ┌──────▼───────┐    ┌──────────┴────┐
              │ Memory m-…   │───▶│ Memory m-007  │
              │ "approach X  │    │ "approach X   │
              │  works"      │    │  doesn't work │
              └──────────────┘    │  for case Y"  │
                                  └───────────────┘
```

The graph is queryable: "show all items justified by intent i-001,"
"show memory entries that defeated other entries," "show backlog
items with no active justifying intent" (orphans), etc.

### Catalog as a knowledge graph

The same engine consumes the catalog: `ComponentEntry` is a node
kind, `related-components.yaml` edges are typed edges (per
component-ontology's `EdgeKind`), and queries like "components on the
path from A to B" or "SCCs in the dependency graph" are
implementations of generic graph queries.

The plan KG and catalog KG live in separate stores but share the same
engine, the same query language, and (where useful) can be unioned
for cross-cutting queries — e.g., "memory entries grounded in
components touched by this plan" requires joining plan memory with
catalog component data.

### Datalog-style inferencing

The engine supports declarative rules, evaluated as Datalog over the
graph:

```
% A backlog item is orphaned if no justifying intent is active.
orphaned(Item) :- backlog_item(Item, _),
                  not exists(I, serves_intent(Item, I), intent(I, active)).

% A memory entry is suspect if any code-anchor's SHA has changed.
suspect(Entry) :- memory_entry(Entry, _),
                  justification(Entry, code_anchor(_, Path, _, OldSha)),
                  current_sha(Path, NewSha),
                  OldSha \= NewSha.

% Component is reachable from another via dependency edges.
reachable(A, B) :- depends_on(A, B).
reachable(A, C) :- depends_on(A, B), reachable(B, C).
```

Rules are domain-specific (different for plan KG vs catalog KG) but
the engine and rule syntax are shared. Triage's "gap detection" is a
Datalog query; reflect's "suspect entry" check is a Datalog query;
catalog's "components in this SCC" is a Datalog query.

The Rust implementation candidate is `ascent` (or similar) — embedded
Datalog with stratified negation, evaluated bottom-up. The user-facing
layer is rules-as-code, not a string DSL, in v1.

### General graph queries

Beyond Datalog, the substrate exposes graph algorithms:

- Shortest path between two nodes.
- Subgraph extraction (BFS at depth N from a node).
- Strongly-connected components.
- Articulation points / bridges.
- Topological sort over a DAG view.

These are convenient operations for which Datalog is awkward
(recursive Datalog can express path queries, but graph-algorithm-
shaped queries are clearer as imperative API calls). Both API styles
coexist; the user/agent picks the right tool.

### Library extraction

The substrate is built as an in-project workspace crate named
`knowledge-graph`, with eventual extraction into a separately-
published crate consumable by ravel-lite, atlas-contracts, and Atlas
itself when it runs as an actor in Ravel. v2.0 keeps the crate
in-tree to keep iteration cheap; extraction is a follow-up once the
shape is stable. The crate provides:

- Typed item/edge schema (generic over kind).
- YAML serde with the canonical shape.
- Status mutation API (with cascade).
- Datalog engine binding.
- Graph algorithm API.
- Query CLI helpers (so consumers like ravel-lite can plug into a
  unified query verb cheaply).

Domain consumers register their item kinds, status vocabularies, and
justification kinds; the engine handles the rest.

The migration order: build the library against ravel-lite's plan KG
(intents, backlog, memory, findings) first, since that's the one
under active design. Catalog KG comes second as Atlas's outputs
already match the shape (ComponentEntry as node, related-components
edges as edges). Atlas-as-actor in Ravel comes later still.

### CLI surface for queries

The catalog already has `ravel-lite atlas <subcmd>`. The plan KG gets
a parallel surface — and over time, both should merge into one query
verb:

```
ravel-lite query catalog "..."        # query the catalog KG
ravel-lite query plan <plan> "..."    # query the plan KG
ravel-lite query unified <plan> "..." # join plan + catalog views
```

For v1, `ravel-lite atlas <subcmd>` (catalog) and a small
`ravel-lite plan <subcmd>` (plan KG inspection) are sufficient; the
unified surface is a future extension.

## Phase cycle (restructured)

```
TRIAGE → WORK → ANALYSE-WORK → REFLECT → [boundary] → TRIAGE → ...
```

Triage opens the cycle as true triage in the medical sense. Reflect
closes it. There is no dream phase in the cycle.

### TRIAGE (headless, start of cycle)

Reads: plan intents, plan memory (active entries), backlog,
`focus-objections.yaml` from the previous work phase (if present),
currently mounted targets, code in those targets, the catalog graph
(on demand).

Does:

1. **Intent hygiene.** Walk active intents; check justifications;
   mark intents `satisfied` (when serving items are done and code
   reflects the goal), `defeat` intents now wrong, `supersede` with
   refined ones. Act on intent-relevant items in
   `focus-objections.yaml`.
2. **Defeat cascade.** Mechanically propagate intent status changes
   through `serves-intent` edges to backlog items. Run by the runner,
   not the LLM.
3. **Backlog hygiene.** Walk backlog items; mark items `done` when
   the code reflects them; fix references that no longer resolve;
   update `blocked_by` dependencies; act on `skip-item` objections.
4. **Gap detection.** Identify under-served intents (active intents
   with no active serving items) and orphaned items (active items
   with no active justifying intent). For each: add items / defeat
   intent; re-attribute item / defeat item.
5. **Reality check.** For surviving items, confirm via code reading
   and graph queries that the item is still the right thing.
6. **Focus selection.** Choose one target component for this cycle
   (multi-target only when the work is genuinely atomic across
   components in the same git root) and pick the backlog items to
   attempt — informed by intent priority and item dependencies.
7. **Mount requests.** If reasoning identifies a needed component
   that isn't mounted, write a `target-requests.yaml` entry and
   stop the cycle (next cycle starts after mounting).
8. **Output.** Write `this-cycle-focus.yaml`, updated `intents.yaml`,
   updated `backlog.yaml`. Consume and remove `focus-objections.yaml`.

`this-cycle-focus.yaml`:

```yaml
target: atlas:atlas-core
backlog_items: [t-001, t-005]
notes: |
  t-005 depends on t-001 completing first; do them in that order.
  Item t-007 is deferred — its code area is mid-refactor on main.
```

### WORK (interactive)

The agent owns the terminal. Reads `this-cycle-focus.yaml`, the focus
target's memory at `<worktree>/<component-path>/.atlas/memory.yaml`,
plan memory, the relevant backlog items, the catalog graph on demand.

CWD: `<context>/plans/<plan>/`. Writable trees exposed via
`--add-dir`: the plan dir and every mounted target's worktree.

Edits land in two physically separate trees:

- Plan state at `<context>/plans/<plan>/`.
- Code at `<plan>/.worktrees/<repo>/`.

Within-plan subagents (the Claude Code Task tool) are preserved —
the existing TUI's concurrent-agent rendering serves them.

#### Escalation via `focus-objections.yaml`

The work phase may produce no code commits and still complete a valid
cycle — when triage's focus turns out to be wrong. The agent writes
`focus-objections.yaml` declaring its objections; the next triage
reads it and acts.

```yaml
objections:
  - kind: wrong-target                     # focus should be a different component
    suggested_target: atlas:atlas-ontology
    reasoning: "Edit needs ontology-side change first"
  - kind: skip-item                        # specific item is not ready
    item_id: t-007
    reasoning: "Blocked on a refactor in main we haven't synced"
  - kind: premature                        # whole focus is premature
    reasoning: "Need to understand X before editing Y. See memory m-…"
```

A cycle that writes only objections + memory entries (no code commits)
is valid. Triage acts on the objections at the start of the next
cycle. Reasoning text flows verbatim into next-triage's prompt;
machine fields drive mechanical hygiene.

### ANALYSE-WORK (headless)

Reads the diff in each worktree against the per-worktree baseline,
plus the work-phase session log. Writes `commits.yaml` partitioning
edits into ordered commits, scoped by path.

```yaml
commits:
  - target: atlas:atlas-core           # implies worktree + pathspec scoping
    paths: ["crates/atlas-core/src/**"]
    message: "feat(atlas-core): add foo edge kind"
  - target: atlas:atlas-ontology       # same worktree (atlas), different subtree
    paths: ["crates/ontology/src/**"]
    message: "feat(ontology): register foo edge kind"
```

The `target:` field is a ComponentRef. The applier resolves it to
`(working_root, path_segments)`, runs `git add -- <pathspec>` and
`git commit` in the working_root.

A work phase that produced no code edits (only objections + memory)
yields no `commits.yaml`; analyse-work writes only the session log
and proceeds.

### REFLECT (headless)

Three responsibilities:

1. **Distil session learnings** into new memory entries with
   structured justifications and component attribution. Process
   learnings (about the plan's own dynamics rather than any
   component) get `attribution: plan-process` and stay in plan
   memory only — they don't promote.
2. **Bounded truth maintenance** on recent plan memory entries:
   re-validate justifications affected by this cycle's changes,
   mark defeated entries, supersede contradicted ones.
3. **Intent-trajectory check.** Walk this cycle's commits and session
   log against active intents. Flag drift: work that doesn't serve
   any active intent, or active intents not served by recent work.
   Output is part of reflect's memory, not a hard fail — the user
   may have legitimate reasons for the drift, but surfacing it
   prevents silent drift over many cycles.

Reflect operates on plan memory only. Component memory edits happen
at plan finish (promotion) or via curate.

### Phase boundaries

Between phases, the runner mechanically:

- Drains `target-requests.yaml` if present (mounts new worktrees,
  attaches new components to existing worktrees when same git root,
  updates `targets.yaml`).
- Applies `commits.yaml` if present.
- Runs the defeat cascade over plan KG when status changes occurred.
- Commits plan-state changes (in `<context>/plans/<plan>/`) to the
  ravel context's git history with the standard
  `run-plan: <phase> (<plan>)` message. Targets first, context last.
- Saves baselines into `<phase>-baseline.yaml` (per-worktree map).

## Targets and worktrees

### Mounting

For each mounted target:

- `working_root` = `<plan>/.worktrees/<repo_slug>/`.
- Branch = `ravel-lite/<plan>/main` (plan-namespaced; one branch per
  repo).
- Created via `git worktree add <working_root> -b <branch>` from
  HEAD-of-default-branch in the source repo.

Multiple components in the same repo share a worktree. The
materialiser groups targets by their git root and creates one
worktree per distinct git root.

### Dynamic mounting

The agent writes `target-requests.yaml` when it discovers a needed
component:

```yaml
requests:
  - component: atlas:atlas-ontology
    reason: Need to register a new edge kind for foo
```

At the next phase boundary, the runner reads the file:

- If the requested component shares a git root with an already-
  mounted target, no new worktree is created — just attaches the
  component to the existing worktree (added to `targets.yaml`).
- Otherwise creates a new worktree.
- If the component isn't in any registered repo's `.atlas/`,
  errors with a clear message ("component `xyz` not registered —
  run Atlas in `<repo>` and re-attempt"). No silent failures.

The file is consumed and removed.

### Pure isolation against main

Plan branches do not auto-rebase. `main` advances independently;
the plan operates on its own snapshot of reality. Merge debt is
paid at finish.

`survey` may show "branch is N commits behind main" per target as a
soft signal of accumulating staleness, but does not act on it.

## Recovery from interrupted cycles

A `Ctrl-C` mid-cycle (or any other interruption) leaves the system
between phases. The recovery policy is **restart the cycle**, not
resume:

- Worktrees with uncommitted edits: their dirty state is preserved.
  The next cycle's analyse-work will pick it up if the next work
  phase doesn't first overwrite it.
- Plan dir with uncommitted state changes (memory, backlog, etc.):
  same — preserved as-is.
- Scratch files (`commits.yaml`, `target-requests.yaml`,
  `focus-objections.yaml`) that were partly written: ignored;
  triage will produce fresh ones for the new cycle.

The next `ravel-lite run <plan>` invocation always starts with
triage, sees the world as it is (dirty trees included), and proceeds
from there. Triage's hygiene step naturally folds in any
interrupted-cycle artefacts.

This intentionally avoids resume-state machinery. The system is
always in one of two states: "between cycles" or "in a cycle." A
killed cycle lands you "between cycles"; the next run starts afresh.
The session log of the interrupted cycle is preserved as evidence —
analyse-work and reflect in the next cycle can reference it.

## Memory: plan-transient and component-durable

### Two layers

- **Plan memory** — transient, lives in `<plan>/memory.yaml` as TMS
  items, consolidated and dispersed at plan finish.
- **Component memory** — durable, lives in
  `<component-path>/.atlas/memory.yaml` in the codebase, version-
  controlled with the component, also TMS items.

### Reflect's role on memory (bounded TMS)

For each entry potentially affected by this cycle's changes:

- **Code-anchor checks (mechanical):** does the path exist? Is the
  SHA at assertion still the SHA at that path? If churned, mark
  *suspect*.
- **LLM re-evaluation:** for suspect entries and for entries with
  rationale-only justifications, re-read the relevant code and
  decide: still true / refined / defeated.
- **Authoring discipline:** new entries must include explicit
  justifications. Two contradictory active entries is a bug.

Bounded scope: last K cycles or last N entries. Cheap by design.

### Curate (unbounded TMS, separate process)

`ravel-lite curate` runs across the whole context — every active
plan's memory and every component memory in every registered repo's
`.atlas/`. Validates entries against current code; produces a
`curate-report.yaml` of discrepancies for the user to review.

Runs on the user's schedule (manual, periodic), not in the plan
cycle. Plan execution never blocks on curate.

### Promotion at plan finish

Triggered by `ravel-lite finish <plan>`:

1. The runner invokes a `promote` headless phase. Inputs: the plan's
   full memory.yaml, the catalog (via tool), the plan's intents,
   the plan's targets.
2. The agent emits `promotion-spec.yaml`, grouping promotions by the
   intent they served:

```yaml
promotions:
  - intent_id: i-001                          # the satisfied intent
    entries:
      - entry_id: m-2026-04-26-0001
        target_component: atlas:atlas-ontology
      - entry_id: m-2026-04-26-0002
        target_component: cross-root          # cross-git-root learning
  - intent_id: i-002                          # defeated intent
    entries:
      - entry_id: m-2026-04-26-0010           # "we tried X, it's wrong because Y"
        target_component: atlas:atlas-core    # still valuable as memory
```

3. The runner mechanically applies the spec: for each entry, append
   to the target component's `.atlas/memory.yaml`. `cross-root`
   entries are duplicated to `<context>/cross-root-memory.yaml` (or
   stored on one side of the relevant edge, when atlas-contracts
   provides edge annotations).
4. Promotions land as commits in their target repos on the plan
   branches, alongside the code. PR/merge ships memory and code
   together.

The "highest-level component within a git root" rule maps onto
walking the `parent` chain in components.yaml: an entry that spans
multiple components within one repo lands at their lowest common
ancestor, not at the repo root.

A confirm-before-commit step on the proposed promotion-spec is run
by default; `--yes` skips it for non-interactive workflows.

### Continuous attribution

To keep promotion mechanical, attribution happens at *write* time
(in reflect), not at finish. New plan memory entries carry their
target component when authored. Promotion at finish is grouping
already-tagged entries — no fresh reasoning. Entries the agent can't
attribute go to `attribution: null`; the user reviews these at
finish via the confirm prompt.

## Catalog as graph (graph-RAG)

The catalog is the union of `.atlas/components.yaml` and
`.atlas/related-components.yaml` across registered repos, viewed as
a graph using the same TMS+KG substrate as plan state. Components
are nodes; edges are typed (per component-ontology). The graph is
queryable on demand.

### Cross-repo edges

Cross-repo edges are first-class. An edge from
`atlas:atlas-core` to `ravel-lite:phase-loop` lives in atlas's
`.atlas/related-components.yaml` (or ravel-lite's, or both), with
`from`/`to` fields carrying ComponentRefs. The graph view
deduplicates and unions edges across repos.

### CLI surface

```
ravel-lite atlas list-repos
ravel-lite atlas list-components [--repo R] [--kind K]
ravel-lite atlas describe <component-ref>
ravel-lite atlas edges <component-ref> [--in|--out|--both]
ravel-lite atlas neighbors <component-ref> [--depth N]
ravel-lite atlas path <from> <to> [--max-hops N]
ravel-lite atlas scc                          # strongly-connected components
ravel-lite atlas roots                        # git roots / repos
ravel-lite atlas summary [--repo R]           # counts, top-level shape
ravel-lite atlas memory <component-ref>       # read .atlas/memory.yaml
ravel-lite atlas memory <component-ref> --search <term>
ravel-lite atlas freshness                    # last-Atlas-run signal per repo
ravel-lite atlas help
```

The agent invokes these via the existing Bash tool. No MCP server
required for v1.

### Caching and freshness

The graph store is loaded into memory at runtime from the union of
`.atlas/components.yaml` files. Per-file fingerprint by
(path, mtime); skip reloads on unchanged files. Atlas reruns are
infrequent, so cache hit rate is near-100% in steady-state.

`freshness` returns "Atlas last ran N days ago against this repo."
The agent can use this as a soft signal; high-stakes queries can
opt into `--require-fresh` to error if recent code changes haven't
been reindexed.

### Forward path to Ravel as actor system

When Atlas eventually runs as an actor in Ravel (the Elixir system),
`ravel-lite atlas <cmd>` becomes a thin client routing to the
service. The agent's mental model — "I have query tools, I shell
out to them" — doesn't change. ASTs and finer-grained graph nodes
extend the surface as new subcommands.

## Commits (two streams)

A single phase boundary may produce commits in two distinct git
identities:

- **Plan-state commits** to the ravel context, scoped by pathspec
  to `plans/<plan>/`. Standard message: `run-plan: <phase>
  (<plan>)`. Land on the user's checked-out branch in the context
  (typically `main`).
- **Work commits** to each mounted target's working_root, on
  `ravel-lite/<plan>/main`. One or more commits per worktree per
  cycle, partitioned by `commits.yaml`.

No atomicity across repos — git makes that impossible. Ordering:
target commits first (substantive code), context commit last
(records that the work happened). On a target-commit failure, the
context hasn't recorded false-positive progress; on a context-
commit failure, target work is preserved and the next phase
catches up.

## Plan lifecycle

### `ravel-lite create <plan>`

No component required. Validates `<context>/plans/<plan>/` doesn't
exist. Spawns a headful claude session with the catalog query tools
available. The session has three deliverables:

1. **Intent articulation.** The agent dialogues with the user to
   draft `intents.yaml` — initially 1–5 strategic intents that the
   plan exists to pursue. Intents are TMS items with explicit
   justifications, even at create time (rationale entries linking
   to the user's stated reasons; external entries linking to issue
   trackers when applicable).
2. **Target proposal.** The agent proposes initial mounted targets
   via `target-requests.yaml` based on which components the intents
   reach. The user reviews and confirms before ravel-lite mounts.
3. **Anchor capture.** Components mentioned during the conversation
   that the plan likely *reads* but doesn't *edit* are recorded as
   anchors in `phase.md` — graph-RAG starting points for later
   triage cycles.

`phase.md` itself becomes a rendered overview of the plan (intent
summary + anchors + housekeeping metadata), not the canonical
intent source.

### `ravel-lite run <plan>`

Enters the phase cycle. First cycle's triage handles the bootstrap
case (empty backlog → triage reads intents and generates an initial
backlog of items justified by them).

### `ravel-lite triage <plan>` / `reflect <plan>` / etc.

Each phase is invokable standalone for maintenance and debugging:
re-plan after main has advanced (`triage`), re-distil memory after
manual edits (`reflect`), force a curate pass (`curate`). Each
standalone invocation runs its own phase boundary, so plan-state
changes are committed to the context as usual.

### `ravel-lite sync <plan> --from <other-plan>`

Explicit mechanism for solo devs running multiple plans in
parallel. For each shared target, runs `git merge <other>'s plan
branch` in `<plan>`'s worktree (on `<plan>`'s branch). Targets
unique to `<other>` are mounted into `<plan>` first. Conflicts
surface in the worktree for the user to resolve.

Replaces "direct mode" target types from earlier drafts. Worktree
isolation is preserved by default; visibility is on-demand and
audited via merge commits.

### `ravel-lite finish <plan> [--pr | --merge]`

A plan is *complete* when every intent has `status: satisfied |
defeated | superseded`. Backlog drainage alone doesn't mark
completion — the criterion is intent-shaped.

`finish` promotes plan memory into component memory (per *Promotion
at plan finish*), groups promotions by satisfied/defeated intent
(memory entries that arose pursuing one intent travel together),
then:

- `--pr`: pushes each target's plan branch and opens a PR per
  target repo. Plan dir stays in `plans/` until all PRs land, then
  moves to `archive/`.
- `--merge`: fast-forwards each target's main onto its plan branch
  locally, deletes plan branches, removes worktrees, archives the
  plan.

Default to `--pr` for team-aware workflows; `--merge` is the
solo-dev shortcut.

### Archive

`<context>/archive/<plan>/` retains the plan's `phase.md`,
`intents.yaml`, `backlog.yaml`, and `memory.yaml` (post-promotion,
mostly `discard`-tagged entries) for audit. The intent record —
including defeated/superseded ones — preserves the plan's history
of what it tried to do and why approaches were ruled out. Worktrees
are removed.

## CLI surface (full)

```
ravel-lite init                                 # scaffold ravel context
ravel-lite create <plan>
ravel-lite run <plan>

ravel-lite triage <plan>                        # standalone phase invocations
ravel-lite reflect <plan>
ravel-lite curate                               # context-wide truth maintenance

ravel-lite sync <plan> --from <other-plan>
ravel-lite finish <plan> [--pr | --merge] [--yes]

ravel-lite repo add <name> --url <u> [--local-path <p>]
ravel-lite repo list
ravel-lite repo remove <name>

ravel-lite atlas <subcommand>                   # see Catalog as graph
ravel-lite plan <subcommand>                    # query/inspect plan KG
ravel-lite query <plan>                         # ad-hoc unified queries (future)
ravel-lite reindex <repo>                       # shell out to Atlas

ravel-lite survey                               # multi-plan status
```

## Findings inbox

`<context>/findings.yaml` is a context-level inbox of TMS items.
Triage and reflect write to it when the agent observes something out
of scope for the current plan:

```yaml
findings:
  - id: f-2026-04-26-0001
    kind: finding
    claim: "The lifecycle scope of edges is implicit; might warrant explicit modelling"
    justifications:
      - kind: rationale
        text: "Came up while investigating intent i-003"
    status: new                                 # new | promoted | wontfix | superseded
    component: atlas:atlas-ontology             # optional attribution
    raised_in: plan/foo
    raised_at: 2026-04-26T...
```

Nothing reads from `findings.yaml` during plan execution. The
`survey` command surfaces it. The user processes findings out of
band — promoting `new` findings into actual plans (creating an
intent in the new plan and marking the finding `promoted`), filing
bugs externally, deciding concerns are `wontfix`.

This **replaces** the existing cross-plan `subagent-dispatch.yaml`
mechanism. Triage is now purely advisory cross-plan: it adjusts its
own plan's backlog and writes findings, but never initiates work in
another plan. The only cross-plan effect is through the inbox,
mediated by the user.

## What this replaces or removes

Compared with `architecture.md`:

- `<project>/LLM_STATE/<plan>` model → plans live at
  `<context>/plans/<plan>` in a single ravel context.
- `projects.yaml` (project-as-grand-context) → `repos.yaml`
  (component-as-unit-of-work, repo-as-physical-shell).
- `project_root_for_plan` path math → `Target` abstraction with
  per-target `working_root` and `path_segments`.
- Subagent dispatch fan-out across plans → findings inbox +
  within-plan subagents only.
- Catalog rendering into prompts → graph-RAG via
  `ravel-lite atlas <q>` CLI.
- Dream as in-cycle phase → curate as separate process; reflect
  takes on bounded TMS + intent-trajectory responsibilities.
- End-of-cycle triage (backlog adjust, dispatch) → start-of-cycle
  triage (true triage in the medical sense).
- Direct-mode targets → worktree-only with explicit
  `ravel-lite sync` for cross-plan visibility.
- Free-text memory.md → structured TMS memory.yaml (md rendered
  from yaml for readability).
- `<phase>-baseline` single SHA → `work-baseline.yaml` per-worktree
  SHA map.
- Free-form `phase.md` prose intent → structured `intents.yaml`
  (TMS items with explicit justifications); `phase.md` becomes a
  rendered overview.
- Implicit forward motion in work phase → explicit escalation via
  `focus-objections.yaml` (a cycle may produce only objections and
  memory).
- Per-kind ad-hoc state files → unified TMS substrate, extracted
  into a reusable crate (intents/backlog/memory/findings as item
  kinds, catalog as a separate KG using the same engine).
- Implicit "cycle resume" semantics → restart-cycle on interruption
  (no resume machinery).
- Intent encoded implicitly in backlog items → intent and task as
  separate, related TMS kinds with `serves-intent` justification
  edges; defeat cascade keeps backlog honest.

## Settled decisions

Resolved during the kick-off of architecture-next migration work
(2026-04-28). Recorded here so the doc stays the canonical source.

- **TMS/KG crate name and home.** Crate is named `knowledge-graph`
  and ships as an in-project workspace member of the ravel-lite
  repository for v2.0. Future extraction into its own published
  crate (consumable by atlas-contracts and Atlas) is deferred until
  the shape stabilises.

- **Datalog engine.** `ascent`. At the ravel-lite scale (plan KGs
  with tens-to-hundreds of items, catalog KGs with low-thousands of
  components), batch recompute is microseconds-to-fractions-of-a-
  second, so differential-dataflow's incrementality pays no
  dividend and adds substantial API/runtime complexity. `crepe` is
  similar in shape to ascent but with less momentum.

- **v1 → v2 cutover policy.** Hard cutover: the v2 binary does not
  run v1 plans. Migration is in-scope for v2.0 via
  `ravel-lite migrate <old-plan-path>`; running ravel-lite 2.x
  against an unmigrated `LLM_STATE/<plan>` directory is an error.

- **Legacy memory/intent backfill.** One-shot LLM-driven extraction
  during `ravel-lite migrate`: the agent reads `phase.md` and
  recent backlog and emits an `intents.yaml`, then re-reads
  `memory.md` and emits a `memory.yaml` with best-effort
  justifications. Entries the agent cannot attribute receive
  `status: legacy` and are surfaced for the user to curate.

## Open questions

These are the spots where the design is not yet settled:

- **Live-update vs batch reasoning.** The Datalog engine recomputes
  derivations from the current fact base; for small plan KGs this
  is cheap. For large catalog KGs it may need incrementalisation
  — open question whether that complexity is needed at the
  ravel-lite scale or only at Atlas-as-server scale.

- **Multi-target-cycle escape hatch semantics.** When a cycle is
  marked multi-target in `this-cycle-focus.yaml`, the work phase
  edits across targets and `commits.yaml` partitions across
  multiple worktrees. Mostly a prompt-rendering question, but the
  agent's mental model needs to stay coherent.

- **Cross-repo memory home.** Duplicate vs context-level vs edge-
  attached. Default to duplicate for v1; migrate to edge-attached
  if/when atlas-contracts provides the slot.

- **Atlas freshness signaling.** The `--require-fresh` semantics —
  what "recent code changes" means, how it's detected, how the
  agent should react to a freshness error.

- **Whether intent-trajectory check is its own phase or part of
  reflect.** Currently designed as part of reflect; if it grows in
  scope (e.g. validating against satisfied-intent criteria), it
  may want its own phase between reflect and the next triage.

- **Rename cascade.** Plan rename, component rename in Atlas, and
  repo rename all need to update references in plans' state files
  and possibly branch names. The unified TMS substrate's
  justification edges (`serves-intent`, `code-anchor` references)
  become new cascade points; design the registry up front per the
  prior `feedback_rename_cascade_registry.md` memory.

- **Concurrent plan execution.** The design assumes plans run
  serially in one TUI process. Two `ravel-lite run` invocations on
  different plans simultaneously is not addressed; would need
  per-plan locks or a shared coordinator.

## Migration sketch

Existing plans live at `<project>/LLM_STATE/<plan>` with free-text
memory and a single-project model. Migration is a one-time per-plan
operation:

1. **Register repos.** For each project hosting an existing plan,
   add an entry to the new context's `repos.yaml` (URL +
   local_path).
2. **Run Atlas.** Index each registered repo so `.atlas/` exists
   with components.yaml. Plans can't start without this.
3. **Move plan state.** Copy `<project>/LLM_STATE/<plan>/` →
   `<context>/plans/<plan>/`. Rename `memory.md` to a legacy
   placeholder; the new `memory.yaml` starts mostly empty.
4. **Extract intent.** A one-shot LLM pass over the existing
   `phase.md` + recent backlog produces `intents.yaml` with
   best-effort intents; backlog items get `serves-intent`
   justifications (or `legacy: true` if unattributable).
5. **Mint targets.** For each plan, write an initial `targets.yaml`
   based on the project's primary component(s) (a one-shot LLM
   pass against the project's components.yaml is the realistic
   approach — the human user reviews).
6. **Mount worktrees.** Initialize worktrees for the targets,
   branching from the project's current main.
7. **Backfill memory** (best effort). Optional one-shot LLM pass
   over `legacy_memory.md` that produces a `memory.yaml` with
   `status: legacy` entries. The user can curate these in their
   own time.

v2.0 is a hard cutover: the v2 binary does not run v1 plans.
Existing plans must be migrated via `ravel-lite migrate
<old-plan-path>` before they can be operated. Migration is a
one-shot, LLM-driven backfill that extracts intent claims from
`phase.md` and reconstructs justifications for legacy `memory.md`
entries; entries the LLM cannot attribute land with `status:
legacy` for the user to curate.
