# R7 — LLM-driven Discovery for Related Projects

**Status:** Shipped. Historical design record.
**Date:** 2026-04-22 (original); superseded on schema vocabulary
2026-04-23.

> **Superseded vocabulary.** When this spec was written, the edge store
> used two kinds (`sibling`, `parent-of`) in a file named
> `related-projects.yaml`. The ontology was bumped to v2 (17 kinds over
> three axes) and the edge store renamed to `related-components.yaml`.
> For the current edge vocabulary, schema, and apply semantics, read
> [`component-ontology.md`](component-ontology.md) — it is authoritative.
> The transition itself is documented in
> [`component-ontology-migration.md`](component-ontology-migration.md).
> This doc is retained for pipeline architecture (two-stage split, cache
> design, concurrency, failure modes), which the ontology docs do not
> cover. Passages that speak about edge kinds, proposal fields, or the
> old file name are marked historical below.

**Related backlog tasks:** R7-design (this spec) — closed; R7
(implementation) — shipped; follow-on ontology-v2 work in
`component-ontology-migration.md` — shipped.
**Depends on:** R5 (global catalog + edge store) — done.

## Problem

When R7 was designed, the global edge list was populated by one of two
mechanisms: (a) explicit `state related-projects add-edge` invocations, or
(b) one-shot migration of legacy per-plan `related-plans.md` files via
`state migrate-related-projects`. Both required the user to already know
the relationships. As the catalog grows (and most catalogued projects
have no plan, so there is no legacy `related-plans.md` to migrate),
manually maintaining this graph does not scale.

R7 adds an LLM-driven discovery mechanism: given the catalog, analyse
every project's source tree and propose edges. The proposals are written
to a review-gate file; a separate `apply` step commits them to the
edge store (today `related-components.yaml`).

> _Historical note._ The v1 migrator (`state migrate-related-projects`)
> was retired at the v2 cutover — see `component-ontology-migration.md`
> §6.1. The `add-edge` escape hatch survives under the v2 CLI verb
> `state related-components add-edge`.

## Scope

### In scope
- A new CLI verb pair: `discover` and `discover-apply`. (Originally
  under `state related-projects`; renamed to `state related-components`
  at the v2 cutover, with the old name retained as a deprecation-window
  alias.)
- Two-stage LLM pipeline: per-project surface extraction (cached) → global
  edge inference (uncached).
- Subtree-scoped git tree SHA as the cache key; works for both top-level
  repos and monorepo subtrees. *(The shipped key combines this with a
  `dirty_hash` over uncommitted state — see §Cache.)*
- Proposal file with rationale; manual review-gate merge into the edge
  store.
- Integration with the existing `projects::run_rename` cascade so cache files
  follow project renames.

### Out of scope (deferred)
- Non-git project support (bail with actionable error for now).
- ~~Dirty-tree analysis — hashing working-tree state rather than bailing
  (bail keeps results reproducible).~~ *Shipped after all — see §Cache
  and §Preconditions. Reproducibility preserved by folding the dirty
  state into the cache key rather than bailing.*
- Auto-scheduling `discover` from a phase (e.g., triage running it
  periodically). Remains user-invoked.
- Confidence-score calibration / auto-apply thresholds. Review-gate covers
  the primary use case; thresholds are premature without calibration data.
- Discovery of edges involving uncatalogued projects (Stage 2 only proposes
  edges between already-catalogued projects).

## Architecture

Two-stage pipeline invoked by a single CLI verb.

```
                ┌─────────────┐
catalog ──────► │  Stage 1    │  N parallel subagents (semaphore-bounded)
(projects.yaml) │  per-project│  ↳ nested subagents at LLM discretion for
                └──────┬──────┘     large projects
                       │
                       ▼
      discover-cache/<name>.yaml  (keyed by subtree state —
                                   tree SHA + dirty hash;
                                   skipped when cache hits)
                       │
                       ▼
                ┌─────────────┐
                │  Stage 2    │  1 LLM call over all N surface records
                │  global     │
                └──────┬──────┘
                       │
                       ▼
         discover-proposals.yaml  (edges + rationale + failures)
                       │
                       ▼        (manual `apply` or `--apply` flag)
         related-components.yaml (merged; idempotent; directional
                                  conflicts reported + rejected)
```

> _Historical note._ The diagram originally named the edge store
> `related-projects.yaml`; the v2 cutover renamed it. Conflict detection
> also narrowed — see §Merge-Apply Policy.

### Why two stages, not one

Each project is read *in isolation* — the Stage 1 subagent knows nothing
about peers. This keeps the per-project cache key trivial (one project's
content hash, nothing else). If peer summaries were part of the subagent's
input, any peer's tree change would invalidate every cache — defeating the
cache's purpose.

Cross-project relationships that neither project explicitly names — e.g.,
projects that communicate via a shared file format or a common network
protocol — cannot be inferred from one project's vantage point. Stage 2 is
the seam where that inference happens: one LLM call over N bounded surface
records is cheap (no deep file reads), and its input is exactly small
enough to fit.

### Why LLM for Stage 2, not Rust

Surface-record matching involves semantic equivalence — "TaskMessage" in
one surface vs. "task record" in another may be the same concept. Rust
rules could match exact strings but would miss these. Stage 2 reasons over
N bounded records, not trees, so the LLM cost is small and bounded.

## Stage 1 — Per-Project Interaction-Surface Extraction

### Working directory and tools
- Subagent CWD: the project's absolute path from the catalog.
- Tool access: `Read` / `Grep` / `Glob` / `Bash` (for `git ls-files` etc.).
- Nested subagent dispatch permitted via prompt concession — the LLM
  decides when a project is too large for one session and dispatches sub-
  subagents for specific subdirectories. Rust does not impose a heuristic;
  the subagent discovers size empirically while reading.

### Surface record schema

```yaml
schema_version: 1
project: <name>            # injected by Rust post-parse (not LLM-emitted)
tree_sha: <sha>            # injected by Rust post-parse
dirty_hash: <sha or "">    # injected by Rust post-parse
analysed_at: <timestamp>   # injected by Rust
surface:
  purpose: <one-paragraph prose>
  consumes_files: [<path-or-glob>, ...]
  produces_files: [<path-or-glob>, ...]
  network_endpoints: [<protocol>://<address-or-description>, ...]
  data_formats: [<name-or-schema-id>, ...]
  external_tools_spawned: [<binary-name>, ...]
  explicit_cross_project_mentions: [<component-name>, ...]
  interaction_role_hints: [<role>, ...]   # closed vocabulary; optional
  notes: <free-form prose>
```

Identity fields (`project`, `tree_sha`, `dirty_hash`, `analysed_at`)
are injected by Rust after parsing the LLM output to prevent the LLM
from claiming a different project name, a stale SHA, or a fabricated
dirty-state fingerprint. `interaction_role_hints` are advisory
self-labels a component's own prose may declare (closed vocabulary in
`InteractionRoleHint`); Stage 2 treats them as priors, not verdicts —
an edge still requires cross-referenced surface-field evidence.

### Delivery mechanism

The subagent writes surface YAML to a predictable tempfile
`<config-dir>/discover-cache/.tmp-<name>-<pid>.yaml`. Rust reads, validates,
populates identity fields, and atomically renames to
`<config-dir>/discover-cache/<name>.yaml`.

## Stage 2 — Global Edge Inference

### Input
All Stage-1 surface records plus the project catalog (names + paths).

### Prompt contract

The Stage 2 prompt instructs the LLM to propose edges from the N bounded
surface records, citing specific surface fields as justification. Every
edge is a tuple `(kind, lifecycle, direction)` annotated with evidence.

The edge-kind vocabulary is **not duplicated here** — it lives
authoritatively in [`component-ontology.md`](component-ontology.md) §5
(17 kinds across seven families), is data-ified in
`defaults/ontology.yaml`, and is spliced into the shipped Stage 2 prompt
via the `{{ONTOLOGY_KINDS}}` token (`defaults/discover-stage2.md`). See
`component-ontology.md` §6 for direction/symmetry rules and §3.2 for the
lifecycle-scope list.

> _Historical note._ The original R7 design offered only two kinds,
> `sibling` and `parent-of`, matching the shipped schema at the time.
> That vocabulary was replaced wholesale at the v2 cutover
> (`component-ontology-migration.md`). Every `sibling` / `parent-of`
> reference elsewhere in this doc should be read as historical.

### Proposal schema

The shipped shape is authoritatively defined in
[`component-ontology.md`](component-ontology.md) §7 (on-disk schema for
the edge store) and in `component-ontology-migration.md` §5.2 (the
Stage 2 prompt's delta against this R7 spec). In summary:

```yaml
schema_version: 2
generated_at: <timestamp>
source_project_states:    # pins the exact input that produced these
  <project>:
    tree_sha: <sha>
    dirty_hash: <sha or "">
  ...
proposals:
  - kind: <kebab-case; see component-ontology.md §5>
    lifecycle: <kebab-case; see component-ontology.md §3.2>
    participants: [<name>, <name>]  # see §6 direction table
    evidence_grade: strong | medium | weak
    evidence_fields: [<field-path>, ...]
    rationale: <prose; must cite specific surface fields>
  ...
failures: []              # populated only when Stage 1 had failures
```

`source_project_states` pins exactly which version of each project
produced the proposals; useful for audit and for detecting stale
proposals if the user lets the file sit across discover runs. It holds
the full cache key (`tree_sha` + `dirty_hash` pair, see §Cache), not
just the committed-tree SHA.

> _Historical note._ The original R7 spec named this field
> `source_tree_shas` with scalar SHA values, and used
> `supporting_surface_fields` for what is now `evidence_fields`. The
> shipped schema_version is `2`. v1 files are read with an empty
> `source_project_states` map (the legacy field is silently ignored) —
> see the test `proposals_file_reads_legacy_source_tree_shas_as_empty_states`
> in `src/discover/schema.rs`.

## Cache

### Location
`<config-dir>/discover-cache/<project-name>.yaml`

One file per project, per-user, alongside `projects.yaml` and
`related-components.yaml`.

### Key

A `(tree_sha, dirty_hash)` pair, both subtree-scoped:

```
rel = <project-path> relative to `git rev-parse --show-toplevel`
tree_sha   = git rev-parse HEAD:<rel>    # empty rel → root tree
dirty_hash = <git hash-object over (git diff HEAD) ++ untracked contents>
             # empty string when the subtree is clean
```

`tree_sha` is git-native and handles both cases identically:
- Top-level project: `rel` is empty; returns the repo root tree.
- Monorepo subtree: returns only that subtree's tree hash. A commit
  touching a sibling subtree does not invalidate this cache entry.

`dirty_hash` captures uncommitted state — staged/unstaged diffs plus
untracked file contents — so that dirty-tree runs can still cache
(rather than bail) but the cache correctly invalidates when the dirty
state changes. Dirty changes in a sibling subtree do not affect this
subtree's hash (pathspec-scoped `git diff HEAD --` and
`git ls-files --others --`).

See `src/discover/tree_sha.rs::compute_project_state` for the
canonical implementation and its tests.

> _Historical note._ The original R7 design proposed a single
> tree-SHA key and bailed on a dirty subtree (below). The `dirty_hash`
> half was added so that mid-iteration subtree analysis is still useful
> — bail was too aggressive for the common "working on the project
> right now" case.

### Hit / miss
- Hit (both `tree_sha` **and** `dirty_hash` match cached values): skip
  Stage 1 subagent; use cached surface as-is.
- Miss or absent: dispatch Stage 1 subagent; write cache on success.

### Rename cascade
`projects::run_rename` cascades into the edge store
(`related-components.yaml` under v2) and also renames
`<config-dir>/discover-cache/<old>.yaml` → `<new>.yaml`, matching the
existing cascade pattern. Cache files survive renames because the tree
SHA is unchanged.

### No Stage 2 cache
Stage 2's input is the set of all N surface records, which changes whenever
any project's tree changes. Caching Stage 2 adds a second invalidation layer
with no meaningful hit rate. Stage 2 runs fresh each `discover`.

## Preconditions and Failure Modes

### Non-git project
Bail at Stage 1 dispatch time with an actionable error naming the project
(e.g., `project 'Foo' at /path is not a git repository — initialise with
`git init` or remove from the catalog`). No cache file written.

### Dirty working tree (subtree-scoped)
Not a bail in the shipped implementation — the subtree's dirty state is
folded into `dirty_hash` (see §Cache). The cache invalidates cleanly
when the uncommitted state changes, and Stage 1 proceeds against the
current working tree. A dirty sibling subtree in the same monorepo does
not affect *this* project's hash.

> _Historical note._ The original design bailed on any dirty subtree
> for reproducibility. That turned out to be too strict — see §Cache for
> the shipped approach.

### Stage 1 subagent failure
Best-effort. On failure:
- No cache file written for that project (existing cache, if any, preserved).
- Failure recorded in the proposals file's `failures:` section with project
  name and error summary.
- Stage 2 still runs over the surfaces that did succeed.
- `discover` exit status is non-zero when any Stage 1 failed, so scripted
  callers notice.
- User can re-run `discover --project <failed-name>` to retry specific
  projects.

### Stage 2 failure
Hard failure. If Stage 2 itself errors, no proposals file is written; cached
Stage 1 results are preserved. User re-runs `discover` (Stage 1 will all be
cache hits on the second run, so the retry is cheap).

## Merge-Apply Policy

### Default: review-gate
`discover` writes proposals to `<config-dir>/discover-proposals.yaml` and
exits. The user reviews the file (including rationale per proposal),
edits if desired, and runs `state related-components discover-apply` to
merge.

### Apply semantics
- Reads `discover-proposals.yaml`.
- For each proposal, invokes `RelatedComponentsFile::add_edge`.
- Already-present edges (canonical-key match — see
  `component-ontology.md` §7.3): silent no-op.
- Directional conflict — the one check performed per
  `component-ontology.md` §7.4: a directed edge proposed in the reverse
  direction of an existing edge at the same `(kind, lifecycle)` is
  reported on stdout, the proposal rejected, the existing edge
  preserved. Apply continues with remaining proposals.
- Cross-kind on the same pair is **not** a conflict — it is expected
  (`component-ontology.md` §3.5): two components may legitimately share,
  e.g., `generates@codegen` and `orchestrates@dev-workflow`. The v1
  "sibling-vs-parent-of" conflict no longer exists, since the vocabulary
  no longer does.
- After apply succeeds, `discover-proposals.yaml` is left on disk so the
  user can `rm` it or keep it for reference. Its `source_project_states`
  map records the cache key each project was at when the proposals were
  generated; a future `discover` run overwrites the file.

> _Historical note._ The original R7 design described a broader
> kind-conflict check (rejecting any edge whose proposed kind
> contradicted an existing kind on the same pair). Under v1's two-kind
> vocabulary this was equivalent to "kinds mutually exclusive per
> pair", which the v2 multiplicity rule explicitly permits.

### `discover --apply` shorthand
Fuses discover + apply for scripted / non-interactive use. Identical
semantics to running them sequentially.

## Concurrency

Stage 1 subagent dispatch is semaphore-bounded using `tokio::sync::Semaphore`.

- Default limit: **4** concurrent subagents.
- Override via `--concurrency <N>`.
- Reuses the existing `JoinSet`-based fanout pattern from
  `src/subagent.rs::dispatch_subagents`.

No configurable default in `config.yaml` for now — YAGNI. Can be added later
if the default proves wrong for most users.

## Project Selection

- Default: all catalogued projects participate. Cache hits skip the LLM
  call but their cached surfaces still flow into Stage 2.
- `--project <name>`: restrict Stage 1 dispatch to just that project;
  Stage 2 still runs over the full catalog's surfaces (cached + freshly-
  analysed). Useful for debugging a single project's extraction or
  retrying after a Stage 1 failure.

## CLI Surface

```
ravel-lite state related-components discover
    [--config <dir>]
    [--project <name>]
    [--concurrency <N>]
    [--apply]

ravel-lite state related-components discover-apply
    [--config <dir>]
```

The `discover-apply` sub-verb (rather than nested `discover apply`) keeps
the clap structure flat and matches the existing flat-verb convention
elsewhere in the CLI surface.

> _Historical note._ The original R7 spec rooted these verbs under
> `state related-projects`. That root is preserved for one release cycle
> as a deprecation-window alias that forwards to
> `state related-components` with a stderr warning
> (`src/main.rs:757`); it will be removed per the policy in
> `component-ontology-migration.md` §6.4.

## Rust Module Layout

*As shipped — deltas from the original spec called out inline.*

- **Pipeline:** `src/discover/{mod,cache,stage1,stage2,apply,schema,tree_sha}.rs`.
  Shipped split, not the single-file form the original spec proposed —
  the anticipated split point was reached during implementation.
- **Types:** `SurfaceRecord`, `SurfaceFile`, `ProposalRecord`,
  `ProposalsFile`, `InteractionRoleHint` in `src/discover/schema.rs`.
  The core ontology types (`Edge`, `EdgeKind`, `LifecycleScope`,
  `EvidenceGrade`, `RelatedComponentsFile`) live separately in
  `src/ontology/` — that module is slated for eventual extraction to a
  workspace crate (`component-ontology-migration.md` §7). Serde derive
  throughout. `schema_version` field on each on-disk file.
- **CLI:** `RelatedComponentsCommands` in `src/main.rs` with `Discover`
  and `DiscoverApply` variants; routed via
  `dispatch_related_components`. The legacy `RelatedProjectsCommands`
  name is retained for one release cycle as the deprecation-window
  alias.
- **Prompt templates:** `defaults/discover-stage1.md` and
  `defaults/discover-stage2.md`. Substituted via the existing
  `substitute_tokens` pipeline. Stage 2 picks up `{{ONTOLOGY_KINDS}}`
  rendered from `defaults/ontology.yaml`
  (`component-ontology.md` §8).
- **Agent trait:** no changes. Stage 1 **does not** use
  `Agent::dispatch_subagent`; it spawns `claude -p` directly via a
  dedicated `spawn_claude_with_cwd` helper in `src/discover/stage1.rs`.
  This closes the original spec's "widen the target parameter" open
  question — the shipped resolution is simply to bypass the Agent trait
  for project-rooted dispatch, since no plan-level state is involved.
- **Cascade:** `projects::run_rename` cascades into
  `related_components::rename_component_in_edges` and
  `discover::cache::rename` (see `src/projects.rs:235–236`).

## Testing Strategy

- **Unit:** surface/proposal YAML round-trips; cache hit/miss logic; tree-
  SHA computation for monorepo subtree and top-level cases; rename-cascade
  coverage for cache files.
- **Integration:** fake-agent (`ContractMockAgent`-style) emits canned
  surface YAML per project → validates end-to-end cache → Stage 2 →
  proposals file writeback. Mirrors the existing integration-test pattern
  in `tests/integration.rs`.
- **No live-LLM test in CI** — matches existing convention (all LLM calls
  are fake-agent-backed in tests).
- **Directional-conflict apply test:** seed `related-components.yaml`
  with a directed edge (e.g. `depends-on(A,B)` at `build`), feed a
  proposal that reverses the participants at the same `(kind,
  lifecycle)`, assert the conflict is reported and the existing edge
  preserved. (The v1 shape of this test — seeding `parent-of(A,B)` and
  proposing `sibling(A,B)` — is obsolete; cross-kind on the same pair
  is no longer a conflict per `component-ontology.md` §3.5.)
- **Failure-tolerance test:** Stage 1 fake-agent fails for one project in a
  three-project catalog; assert the failures section is populated, Stage 2
  runs over the two surviving surfaces, and overall exit is non-zero.

## Evaluation

| Criterion | Estimate |
|-----------|----------|
| **Scales to many catalogued projects** | Yes. Per-project cache means steady-state cost is O(projects-changed-since-last-discover). |
| **Context savings vs manual add-edge** | Substantial. User reviews a pre-populated proposals file with rationale instead of maintaining edges by hand. |
| **Handles emergent relationships** | Yes. Two-stage design catches relationships that neither project explicitly names (shared file formats, shared protocols). |
| **Principle cost** | None. All cache, proposals, and apply state are readable files. No magic. |
| **Implementation cost** | ~800–1200 LOC Rust + 2 prompt templates. Comparable to R5 (~900 LOC). |
| **Test cost** | Moderate. Integration test needs a fake agent that emits structured surface YAML; one-off per-test scaffolding but follows an established pattern. |
| **Risk: LLM calibration** | Addressed by review-gate; bad proposals do not reach `related-components.yaml` without user approval. |
| **Risk: runaway cost on first run** | Bounded by `--concurrency` limit; each Stage 1 run is self-contained. User can interrupt with Ctrl-C; successful Stage 1 writes are already cached. |

## Rollout

| # | Task | Size | Dependencies |
|---|------|------|--------------|
| R7.1 | Schema types, cache read/write, tree-SHA helper, unit tests | small | R5 (done) |
| R7.2 | Stage 1 dispatch loop, fake-agent integration test | medium | R7.1 |
| R7.3 | Stage 2 invocation, proposals writeback | small | R7.2 |
| R7.4 | `apply` sub-verb, kind-conflict handling, integration test | small | R7.3 |
| R7.5 | Rename cascade for cache files; update `projects::run_rename` | small | R7.1 |
| R7.6 | CLI wiring, prompt templates, end-to-end integration test | small | R7.1–R7.5 |

Total: ~6 sub-tasks, each ~one-session-sized. The implementation plan
authored by `writing-plans` will sequence and refine these.

## Open Questions

Resolved during implementation:

- **`Agent::dispatch_subagent` widening.** Resolved by *not* using the
  Agent trait for Stage 1 — the pipeline spawns `claude -p` directly
  via `spawn_claude_with_cwd` (`src/discover/stage1.rs`). No trait
  changes.
- **`src/discover.rs` split point.** Split at implementation time into
  `src/discover/{mod,apply,cache,schema,stage1,stage2,tree_sha}.rs`.
- **Proposals-file lifetime after apply.** Kept on disk (default as
  originally proposed); no user reports of confusion to date.

See `component-ontology-migration.md` for the schema-vocabulary
open-question list that came out of v2 — hyperedges, temporal decay,
per-kind evidence schemas, negative edges, catalog pluralism.
