# R7 — LLM-driven Discovery for Related Projects

**Status:** Proposed. Recommendation: **go**.
**Date:** 2026-04-22
**Related backlog tasks:** R7-design (this spec); R7 (implementation).
**Depends on:** R5 (global `related-projects.yaml` + catalog) — done.

## Problem

The global `related-projects.yaml` edge list is presently populated by one of
two mechanisms: (a) explicit `state related-projects add-edge` invocations, or
(b) one-shot migration of legacy per-plan `related-plans.md` files via
`state migrate-related-projects`. Both require the user to already know the
relationships. As the catalog grows (and most catalogued projects have no
plan, so there is no legacy `related-plans.md` to migrate), manually
maintaining this graph does not scale.

R7 adds an LLM-driven discovery mechanism: given the catalog, analyse every
project's source tree and propose edges. The proposals are written to a
review-gate file; a separate `apply` step commits them to
`related-projects.yaml`.

## Scope

### In scope
- A new CLI verb pair under `state related-projects`: `discover` and
  `discover-apply`.
- Two-stage LLM pipeline: per-project surface extraction (cached) → global
  edge inference (uncached).
- Subtree-scoped git tree SHA as the cache key; works for both top-level
  repos and monorepo subtrees.
- Proposal file with rationale; manual review-gate merge into
  `related-projects.yaml`.
- Integration with the existing `projects::run_rename` cascade so cache files
  follow project renames.

### Out of scope (deferred)
- Non-git project support (bail with actionable error for now).
- Dirty-tree analysis — hashing working-tree state rather than bailing
  (bail keeps results reproducible).
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
      discover-cache/<name>.yaml  (keyed by subtree tree SHA;
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
           related-projects.yaml (merged; idempotent; kind-conflicts
                                  reported + rejected)
```

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
analysed_at: <timestamp>   # injected by Rust
surface:
  purpose: <one-paragraph prose>
  consumes_files: [<path-or-glob>, ...]
  produces_files: [<path-or-glob>, ...]
  network_endpoints: [<protocol>://<address-or-description>, ...]
  data_formats: [<name-or-schema-id>, ...]
  external_tools_spawned: [<binary-name>, ...]
  explicit_cross_project_mentions: [<project-name-or-path>, ...]
  notes: <free-form prose>
```

Identity fields (`project`, `tree_sha`, `analysed_at`) are injected by Rust
after parsing the LLM output to prevent the LLM from claiming a different
project name or a stale SHA.

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
surface records, citing specific surface fields as justification. Two edge
kinds are available, matching the existing `related-projects.yaml` schema:

- `sibling(A, B)` — peer-level shared purpose, protocol, or data format.
  Order-insensitive.
- `parent-of(A, B)` — one project produces artifacts or contracts the other
  consumes. Order-sensitive (parent first).

### Proposal schema

```yaml
schema_version: 1
generated_at: <timestamp>
source_tree_shas:          # pins the exact input that produced these
  <project>: <sha>
  ...
proposals:
  - kind: sibling | parent-of
    participants: [<name>, <name>]  # parent first for parent-of
    rationale: <prose; must cite specific surface fields>
    supporting_surface_fields: [<field-path>, ...]
  ...
failures: []               # populated only when Stage 1 had failures
```

`source_tree_shas` pins exactly which version of each project produced the
proposals; useful for audit and for detecting stale proposals if the user
lets the file sit across discover runs.

## Cache

### Location
`<config-dir>/discover-cache/<project-name>.yaml`

One file per project, per-user, alongside `projects.yaml` and
`related-projects.yaml`.

### Key

Subtree-scoped git tree SHA:

```
rel = <project-path> relative to `git rev-parse --show-toplevel`
tree_sha = git rev-parse HEAD:<rel>    # empty rel → root tree
```

This is git-native and handles both cases identically:
- Top-level project: `rel` is empty; returns the repo root tree.
- Monorepo subtree: returns only that subtree's tree hash. A commit
  touching a sibling subtree does not invalidate this cache entry.

### Hit / miss
- Hit (`tree_sha` matches cached value): skip Stage 1 subagent; use cached
  surface as-is.
- Miss or absent: dispatch Stage 1 subagent; write cache on success.

### Rename cascade
`projects::run_rename` already cascades into `related-projects.yaml`. It
will also rename `<config-dir>/discover-cache/<old>.yaml` →
`<new>.yaml`, matching the existing cascade pattern. Cache files survive
renames because the tree SHA is unchanged.

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
Bail. Uses the existing pathspec-scoped `git::working_tree_status(project_dir)`
so a dirty sibling subtree in the same monorepo does not block *this*
project. Error names which files are dirty.

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
exits. The user reviews the file (including rationale per proposal), edits
if desired, and runs `state related-projects discover-apply` to merge.

### Apply semantics
- Reads `discover-proposals.yaml`.
- For each proposal, invokes `RelatedProjectsFile::add_edge`.
- Already-present edges (canonical-key match): silent no-op (existing
  idempotency).
- Kind-conflict (e.g., proposing `sibling(A,B)` when `parent-of(A,B)`
  already exists, or the reverse): reported on stdout, proposal rejected,
  existing edge preserved. Apply continues with remaining proposals.
- After apply succeeds, `discover-proposals.yaml` is left on disk so the
  user can `rm` it or keep it for reference. The file header records the
  tree-SHA snapshot at proposal time; a future `discover` run overwrites it.

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
ravel-lite state related-projects discover
    [--config <dir>]
    [--project <name>]
    [--concurrency <N>]
    [--apply]

ravel-lite state related-projects discover-apply
    [--config <dir>]
```

The `discover-apply` sub-verb (rather than nested `discover apply`) keeps
the clap structure flat and matches the existing flat-verb convention
elsewhere in the CLI surface.

## Rust Module Layout

- **New file:** `src/discover.rs` — orchestrates the pipeline. Owns the
  Stage 1 dispatch loop, cache read/write, Stage 2 invocation, proposals
  writeback, apply logic. If it grows past ~500 lines, split to
  `src/discover/{mod,cache,stage1,stage2,apply,schema}.rs` following the
  `src/state/` pattern.
- **Types:** `SurfaceRecord`, `SurfaceFile`, `ProposalRecord`, `ProposalsFile`
  in `src/discover.rs` (or `src/discover/schema.rs` post-split). Serde derive.
  `schema_version` field on both files.
- **CLI:** extend `RelatedProjectsCommands` in `src/main.rs` with
  `Discover { .. }` and `DiscoverApply { .. }` variants; route via
  `dispatch_related_projects`.
- **Prompt templates:** `defaults/discover-stage1.md` and
  `defaults/discover-stage2.md`. Substituted via the existing
  `substitute_tokens` pipeline. No new tokens expected.
- **Agent trait:** no changes to `Agent`. Stage 1 dispatch reuses the
  existing `Agent::dispatch_subagent` — the "target plan" parameter is
  widened by convention to accept a project path when no plan is involved.
  (If this proves awkward in practice, a sibling method
  `dispatch_project_subagent` can be added during implementation; the
  decision is deferred to the implementation plan.)
- **Cascade:** extend `projects::run_rename` to also rename cache files
  in `<config-dir>/discover-cache/`.

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
- **Kind-conflict apply test:** seed `related-projects.yaml` with
  `parent-of(A,B)`, feed a proposal of `sibling(A,B)`, assert the
  conflict is reported and the existing edge preserved.
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
| **Risk: LLM calibration** | Addressed by review-gate; bad proposals do not reach `related-projects.yaml` without user approval. |
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

None at spec time. A few sub-decisions are deferred to the implementation
plan because they are low-stakes and best resolved against actual code:

- Whether `Agent::dispatch_subagent`'s target parameter is widened by
  convention or a sibling method is added.
- Exact split point if `src/discover.rs` outgrows a single file.
- Whether `discover-proposals.yaml` is kept or deleted after a successful
  `apply` (default keep; revisit if it causes confusion).
