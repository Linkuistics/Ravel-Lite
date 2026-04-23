# Component Ontology — Migration Plan

**Status:** Migration plan. **Retire and delete this document once the
work described below has shipped and bedded in.**

This document covers the delta between the shipped two-kind model
(`sibling` / `parent-of`) and the ontology specified in
`docs/component-ontology.md`. It is input to planning, not a reference
spec; the final state is owned by that reference doc, and citations
below point into it by section number.

Companion: `docs/r7-related-projects-discovery-design.md` established
the current pipeline and the v1 schema; this document governs the bump
to v2 and all the consumer renames that go with it.

## 1. The problem

### 1.1 Current state — v1 two-kind model

Schema v1 provides exactly two edge kinds:

```rust
// src/related_projects.rs (current)
pub enum EdgeKind { Sibling, ParentOf }
```

- `sibling(A, B)` — unordered peer; shared purpose, protocol, or data
  format.
- `parent-of(A, B)` — ordered; A produces artifacts B consumes.

Every real cross-component coupling collapses onto one of these two
buckets, losing three orthogonal distinctions that matter:

- *When* the coupling is active — build-time codegen vs. runtime IPC
  vs. dev-workflow orchestration.
- *What* is shared — types vs. wire format vs. a whole subprocess
  lifecycle vs. a spec.
- *Which* direction the influence flows — producer → consumer vs.
  orchestrator → orchestrated vs. implementation → spec.

### 1.2 Concrete failure case

R7 smoke testing proposed `parent-of(Ravel-Lite, Ravel)`. Ravel-Lite
does not produce artifacts Ravel links against or reads at runtime — it
emits plan-state YAML *schemas* (codegen) and spawns agents that drive
the loop (dev-workflow). Two relationships at two lifecycle scopes. The
two-kind model could express neither.

The reference ontology resolves this as two edges:

- `generates @ codegen` for the schema flow
- `orchestrates @ dev-workflow` for the agent loop

See `docs/component-ontology.md` §3.5 for the multiplicity rule that
makes this legal.

### 1.3 Naming rationale — projects → components

The shipped naming (`related-projects`, `RelatedProjectsFile`,
`projects.yaml`) was a scope marker from when the catalog was
exclusively whole projects. Components is the general case; projects
are one specialisation. The ontology itself doesn't change when the
unit changes — the same edge kinds apply to crates, services, or
subsystems.

Rename policy (§6) retains the project-catalog filename (`projects.yaml`)
because that catalog is, literally, a list of whole projects —
widening it is a separate concern. The *edge store* is generalised
(`related-components.yaml`) because the edges themselves are not
project-specific.

## 2. Why there is no migrator

`related-components.yaml` (and its v1 predecessor `related-projects.yaml`)
is an **entirely generated artifact** — Stage 2 of the discover pipeline
produces every edge. No human authors rows in it by hand; the
`add-edge`/`remove-edge` CLI is a maintenance escape hatch, not the
primary input mechanism.

Given that, schema migration would be waste: a rule-based v1 → v2
transform is *guessing* at evidence that was never captured in v1 (v1
has no `lifecycle`, no `evidence_grade`, no `evidence_fields`), whereas
a fresh discover run under the v2 prompt produces edges with direct
evidence. The correct upgrade path is therefore: delete the v1 file and
re-run discover.

## 3. User-facing upgrade procedure

1. Delete `<config-root>/related-projects.yaml` (and
   `<config-root>/discover-proposals.yaml` if present — its schema also
   bumps).
2. Run `ravel-lite state related-components discover --apply`.
3. A fresh `<config-root>/related-components.yaml` is produced, with
   every edge carrying `lifecycle`, `evidence_grade`, and
   `evidence_fields` directly from Stage 2 evidence.

The Stage 1 per-component surface cache at
`<config-root>/discover-cache/<name>.yaml` does **not** need to be
deleted — its schema is unchanged, and preserving it keeps the re-run
cheap.

## 4. Loader behaviour for pre-v2 files

- Reading `related-projects.yaml` at the old path: hard error with an
  actionable message pointing at §3.
- Reading `related-components.yaml` with `schema_version != 2`: hard
  error (consistent with existing drift behaviour for other YAML
  files). The error message names the file, the observed version, the
  expected version, and the `discover --apply` command.
- No in-memory upgrade path. No deprecation window on the v1 schema.
- Hand-authored edges (via the `add-edge` escape hatch) are the only
  content that could theoretically be lost across an upgrade. Since
  `add-edge` is not the primary population mechanism, this is a
  documented user responsibility: if a user has hand-authored edges in
  v1 that a discover re-run does not reproduce, they re-apply them
  with v2 `add-edge` invocations.

## 5. Discover pipeline changes

### 5.1 Stage 1 — non-breaking addition

`SurfaceRecord` gains one optional field:

- `interaction_role_hints: [generator, orchestrator, test-harness,
  spec-document, spawner, documented-by, …]` — advisory labels a
  component's own prose declares about itself. Stage 2 still picks the
  kind from cross-referenced evidence; hints are priors, not verdicts.

No existing field is removed or renamed. The surface-record cache key
(subtree tree SHA) is unaffected.

### 5.2 Stage 2 — prompt rewrite

`defaults/discover-stage2.md` is rewritten:

- The "Edge kinds" section is replaced by substitution of
  `{{ONTOLOGY_KINDS}}` rendered from `defaults/ontology.yaml`
  (reference doc §8).
- A new "Decision tree" section explicitly walks the kind-picking
  order:

  ```
  1. Runtime message exchange? (network_endpoints match)
     → communicates-with | calls
  2. Source generation into another tree? (produces_files ∩ sources)
     → generates  @ codegen
  3. Process spawning? (external_tools_spawned ∩ owner)
     → invokes | orchestrates
  4. Library dependency? (manifest evidence)
     → depends-on | links-statically | links-dynamically
  5. Common external spec declared by both?
     → co-implements @ design
  6. Doc-repo relationship?
     → describes
  7. Test harness / fixture provider?
     → tests | provides-fixtures-for  @ test
  8. None of the above + no direct evidence?
     → no edge
  ```

- Output schema updates to match reference doc §7: `lifecycle`,
  `evidence_grade`, `evidence_fields`, `rationale`. Existing
  `rationale` and `supporting_surface_fields` carry over (the latter
  renamed to `evidence_fields`).

### 5.3 Apply phase

`src/discover/apply.rs`:

- Canonical-key check updated per reference doc §7.3 (add lifecycle
  dimension).
- Conflict detection narrowed per reference doc §7.4 (cross-kind
  conflicts gone; reversed-directed-edges check retained).
- Proposals file schema bumps in lockstep.

## 6. Consumer audit and rename policy

### 6.1 Direct consumers

Every known reader / writer of the v1 graph, and what changes:

| Site | File / symbol | v2 change |
|---|---|---|
| Core types | `src/related_projects.rs` | Moves to `src/ontology/`. Module + types renamed (`related_projects` → `ontology`, `RelatedProjectsFile` → `RelatedComponentsFile`, `rename_project_in_edges` → `rename_component_in_edges`). |
| Constant | `RELATED_PROJECTS_FILE` | Renamed `RELATED_COMPONENTS_FILE`; value `related-components.yaml`. |
| Discover Stage 2 output | `src/discover/stage2.rs`, `src/discover/schema.rs` | Emits v2 `ProposalRecord`; proposals-file `schema_version` bumped. |
| Discover apply | `src/discover/apply.rs` | Canonical-key + conflict-detection updates. |
| Discover cache rename cascade | `src/discover/cache.rs` | Unaffected (cache is keyed on component name; rename cascade already handled). |
| CLI | `ravel-lite state related-projects …` | Renamed `state related-components …`. Keep `state related-projects` as a deprecated alias for **one** release cycle, emitting a stderr warning that forwards to the new name. |
| CLI args | `add-edge kind a b` | Extended: `add-edge kind lifecycle a b --evidence-grade … --evidence-field … [--evidence-field …] --rationale …`. `kind` values match reference doc §5. |
| CLI args | `list [--plan]` | Extended: `list [--plan] [--kind X] [--lifecycle Y]` for filtering. |
| Rename cascade | `src/projects.rs` (`run_rename`) | Calls `rename_component_in_edges` instead of `rename_project_in_edges`; cache filename rename unchanged. |
| Legacy markdown migrator | `state migrate-related-projects` | Reads per-plan `related-plans.md` and emits edges. Since v2 loaders reject v1 files, this CLI must either be retired at v2 cutover or taught to emit v2 edges (trivial: it already has enough context to pick `depends-on` / `describes`). Retire by default; reintroduce only if a user asks. |
| Tests | `tests/state_related_projects.rs` | Renamed + extended for the new fields. Fixture edges in existing unit tests (`src/related_projects.rs:606–1077`) become v2. |

### 6.2 Indirect consumers and non-consumers

- `src/prompt.rs` / `read_related_plans_markdown` (`src/main.rs:1083`,
  `src/multi_plan.rs:27`, `src/multi_plan.rs:62`) — reads **per-plan
  markdown** (`related-plans.md`), not the structured graph.
  **Unaffected.** This is the legacy integration that the future
  graph-aware prompt substitution will eventually replace.
- Phase prompts that reference the graph today — **none**. The v2
  schema is thus not breaking any prompt contract today; new prompts
  that consume the graph will adopt v2 directly.
- `projects.yaml` — the component catalog itself. **Unaffected**; its
  schema is independent.

### 6.3 Filename rename

- `<config-root>/related-projects.yaml` — no file moves. The v1 file is
  deleted by the user as part of the §3 upgrade; the v2 file is
  written by `discover --apply` at the new path
  `related-components.yaml`.
- `<config-root>/discover-proposals.yaml` — unchanged filename; only
  its schema bumps. Any residual v1 proposals file is deleted as part
  of the §3 upgrade.
- `<config-root>/discover-cache/*.yaml` — unchanged, retained across
  upgrade to keep the re-run cheap.

### 6.4 Rename policy

- CLI: deprecated alias for one release cycle, then removed.
- Types / constants / modules: no aliases. One Ravel-Lite release ships
  the rename atomically with the schema bump.
- `projects.yaml` catalog: **not renamed**. The catalog is a project
  catalog today; the ontology operating over it is a separate concern.

## 7. Extraction plan

Staged. Do not attempt to extract on day one.

### 7.1 Phase A — internal module (immediate)

Location: `src/ontology/` inside Ravel-Lite. Replaces
`src/related_projects.rs`. Shape matches reference doc §9 already, but
lives as a module crate-internally. Public use within Ravel-Lite only.

Entry criteria: design approved, follow-up implementation task
accepted.

Exit criteria: v2 schema in production; Stage 2 emits v2; all tests
green; at least one full discover → apply cycle produces v2 edges.

### 7.2 Phase B — workspace member crate (medium term)

Move `src/ontology/` → `crates/component-ontology/` in a Cargo
workspace. Still vendored inside Ravel-Lite's repo. No functional
change; the extraction is purely structural.

Entry criteria: Phase A has been stable for ≥1 release; a second
consumer inside Ravel-Lite (e.g., a phase prompt renderer that walks
the graph) is on the near horizon.

Exit criteria: `cargo build -p component-ontology` succeeds
independently; no Ravel-Lite-specific code in the crate; the crate has
no path dependencies on Ravel-Lite's code (only the reverse).

### 7.3 Phase C — published crate or external repo (speculative)

Publish to `crates.io`, or extract to a dedicated repo, when a second
tool outside Ravel-Lite asks for the ontology. Until then, the
workspace-local crate is sufficient and premature publication costs
more than it saves.

## 8. Acceptance checklist

This migration is acceptable when the work ships and:

- [ ] Every kind in reference doc §5 is groundable in ≥1 Stage 1
      surface field.
- [ ] The Ravel-Lite → {Ravel, APIAnyware-MacOS, TestAnyware} smoke-test
      case resolves without `parent-of` and without information loss.
- [ ] A build-time drift test ensures the Rust `EdgeKind` enum and
      `defaults/ontology.yaml` stay in lockstep.
- [ ] A build-time drift test ensures the Stage 2 prompt renders the
      kind list from the ontology YAML, not from hard-coded prose.
- [ ] Loading a v1 file (either at the old `related-projects.yaml`
      path or with `schema_version: 1` at the new path) produces an
      actionable hard error that names the `discover --apply` command
      (§4). No silent upgrade path remains in the loader.
- [ ] SPDX-alignment column in reference doc §5 is accurate — no
      claimed correspondence that SPDX 3.0.1 doesn't actually have.
- [ ] All direct consumers in §6.1 compile against the new types;
      legacy `read_related_plans_markdown` (§6.2) is untouched.
- [ ] The extraction-readiness criteria for Phase B (§7.2) hold for
      `src/ontology/` before it graduates to a workspace crate.

## 9. Sunset

Delete this document (and remove references to it from the backlog
and commit history by simple deletion, not rewrite) once:

1. The acceptance checklist in §8 is fully ticked.
2. No remaining v1 `related-projects.yaml` files are known to exist in
   any user config directory the maintainer can reach.
3. The deprecated `state related-projects` CLI alias has been through
   its one-release window and been removed.

After sunset, the only surviving doc is `docs/component-ontology.md`.
