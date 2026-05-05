# Multi-repo plan build-closure — design

Date: 2026-05-05
Status: Drafted, pending implementation plan.

## Problem

When migration creates a plan, it currently mounts a worktree only for
the single source repo named in the migration (per `apply_targets.rs`,
which loops `mount_target` over the LLM-chosen targets). Many real-world
repos depend on sibling repos via relative paths in their build manifests
(`Cargo.toml: path = "../<sibling>"`). Once the source repo is mounted at
`<plan>/.worktrees/<source>/`, those relative paths resolve to
`<plan>/.worktrees/<sibling>/` — which does not exist, so the build fails.

A plan that needs to operate across multiple repos must therefore mount
worktrees for every repo it transitively depends on at build/link time.
The current code does no such expansion.

## Goals

- Plans become first-class multi-repo: `targets.yaml` is a flat list of
  `(repo_slug, component_id)` rows, and the mounted set is closed under
  build/link dependency edges from the user-chosen targets.
- Migration mounts the full closure eagerly, so the first build in a
  fresh plan succeeds without extra round-trips.
- The work-phase LLM (or a human operator) can extend the plan's
  footprint mid-flight via a CLI verb that runs the same closure logic.
- Atlas is the single source of truth for cross-repo dependency
  knowledge. Ravel-Lite does not parse `Cargo.toml`, `package.json`, or
  any other build manifest.

## Non-goals

- Lazy / build-failure-driven discovery. Closure runs at mount time, not
  in response to build errors.
- LLM-mediated build-sibling discovery. The work-phase prompt is not
  expected to recognise "I need another repo for the build to compile."
  The CLI verb is for *intentional* footprint expansion (e.g. "I now want
  to touch component X in another repo"), not as a workaround for
  missing build deps.
- Pluggable build-manifest parsers. Atlas-only.
- A new `origin: explicit | transitive` field on `Target`. Not added
  until a downstream phase (reflect, finish) demonstrably needs the
  distinction.
- Retiring `target-requests.yaml`. Left intact; the new CLI verb is
  additive. Whether to deprecate later is a separate decision.
- Multi-plan branch / worktree hygiene at finish. Out of scope; the
  existing `architecture-next.md` §Pure-isolation policy still applies.

## Design

### Architecture

A plan is a set of mounted `(repo, component)` targets. The set is
closed under build/link edges from the initial user-chosen targets,
expanded via Atlas's cross-repo edge graph. Two entry points share one
orchestrator:

1. `migrate-v1-v2 → apply-targets` runs the closure once before mounting,
   so the initial plan state already includes every repo the build
   needs.
2. A new CLI verb (`ravel-lite plan add-target <plan-name>
   <repo>:<component>`) calls the same orchestrator with a single new
   ref. The work-phase LLM invokes this directly when it decides to
   extend the plan's scope.

Both paths mount via the existing `mount_target` (`src/state/targets/
mount.rs`), which is unchanged. Worktrees use the plan-namespaced branch
`ravel-lite/<plan>/main`, exactly as today.

### Module layout

- `src/state/targets/closure.rs` — *new*. Pure module. Inputs: an
  initial list of `ComponentRef`s, a cross-repo `EdgeGraph` (built by
  the orchestrator from registered repos' `related-components.yaml`),
  and the (repo_slug, component) iterator already exposed in
  `src/atlas.rs:292+` for host-repo resolution. Output: the closed set
  of `ComponentRef`s, including the initial refs.

  Walks outgoing directed edges of `EdgeKind` in the build set
  (`depends-on`, `links-statically`, `links-dynamically`,
  `has-optional-dependency`) to a fixed point. No I/O. Errors when an
  edge target's host repo is not registered or when a component
  cannot be resolved against any registered repo's index.

  Concrete signature pinned in the implementation plan.

- `src/state/targets/mount_with_closure.rs` — *new*. Orchestrator.
  Inputs: plan directory, context root, initial refs. Loads Atlas
  indices across all registered repos, builds an `EdgeGraph`, calls
  `expand_build_closure`, and loops `mount_target` over the resulting
  set. Skips refs already present in `targets.yaml`. Idempotent.
  Returns the list of `Target` rows that ended up mounted (new + already
  present), so callers can report what landed on disk.

- **CLI surface (revised during implementation):** the existing
  `ravel-lite state targets mount <plan-dir> <repo>:<component>` verb
  is *upgraded* to call `mount_with_closure` instead of `mount_target`.
  Its contract ("mount this target") is preserved; the closure becomes
  an implicit "and what it needs to build." This avoids adding new CLI
  surface for the work-phase LLM to learn — a strict simplification
  over the original "add a new `plan add-target` verb" plan.

- `src/main.rs` — modify the existing `state targets mount` dispatch
  to call `mount_with_closure`. Update the verb's after-help text to
  explain the closure semantics. No new subcommand is added.

  (The original design proposed a new `plan add-target` verb; the
  upgraded existing verb subsumes it cleanly. See "CLI surface"
  above.)

- `src/migrate_v1_v2/apply_targets.rs` — replace the inline
  `mount_target` loop with one call to `mount_with_closure`. The LLM's
  proposed targets are passed in as the initial set; closure expansion
  produces the full mount list.

### Data flow

**Initial mount (migration):**

```
LLM → migrate-targets-proposal.yaml
    → apply_proposal (apply_targets.rs)
       → mount_with_closure(plan_dir, context, refs)
            ├─ load Atlas index across all registered repos
            ├─ build EdgeGraph (existing src/atlas.rs API)
            ├─ expand_build_closure(refs, graph, registry, catalog)
            │     → fixed-point walk over build/link edges
            │     → returns closed Vec<(repo, component)>
            └─ for each ref in closed set:
                  if (repo, component) already in targets.yaml → skip
                  else → mount_target(plan_dir, context, repo, component)
                          → appends Target row
```

**Mid-plan extension:**

```
LLM (or user): ravel-lite plan add-target <plan-name> <repo>:<component>
            → resolve plan-name → plan_dir
            → verbs::add_target(plan_dir, context, "repo:component")
                  → parse reference
                  → mount_with_closure(plan_dir, context, [ref])
                  → same closure walk as above
```

### Closure semantics

- **Initial set:** the refs supplied by the caller (LLM proposal during
  migration, or the single ref of the CLI verb).
- **Edge filter:** `EdgeKind` ∈ {`depends-on`, `links-statically`,
  `links-dynamically`, `has-optional-dependency`}. Other edge kinds
  (`calls`, `invokes`, `embeds`, `tests`, etc.) are ignored at this
  layer — the LLM uses the CLI verb to bring those in intentionally.
- **Direction:** outgoing only. We follow what the target depends on,
  not what depends on the target.
- **Resolution:** edge participants are bare component ids. Host-repo
  resolution reuses the (repo_slug, component) iterator in
  `src/atlas.rs:292+` and the disambiguation already hardened in
  `migrate-targets` (memory note: "Harden migrate-targets against
  component-id ambiguity").
- **Termination:** fixed-point iteration. A worklist seeded with the
  initial set; each pop expands by one hop; results de-duplicated;
  iteration stops when a pass adds no new refs.

### Error handling

- **Closure references an unregistered repo.** `ErrorCode::NotFound`:
  > component `X` (host repo `Y`) is needed by the build closure but `Y`
  > is not in `repos.yaml`. Add it with
  > `ravel-lite repo add Y --url <url> --local-path <path>` and retry.

- **Closure references an unknown component.** `ErrorCode::NotFound`:
  > the build closure references component `X` but no registered repo's
  > `.atlas/components.yaml` contains it. Either an edge in
  > `<repo>/.atlas/related-components.yaml` is stale (re-run
  > `atlas index <repo>`), or the component was renamed.

- **Stale Atlas index on a host repo.** Bubble through `mount_target`'s
  existing `ATLAS_COMPONENTS_REL` not-found path; the closure walker
  prepends the closure-context message naming the host repo.

- **Plan-branch already exists.** Unchanged — `mount_target` already
  emits the actionable cleanup error (see `mount.rs:188`).

- **Malformed `<repo>:<component>` CLI argument.**
  `ErrorCode::InvalidInput` via the existing `cli/error_*` envelope.

- **Closure on a fresh-target re-mount that's already in `targets.yaml`.**
  Not an error. The orchestrator skips already-mounted rows; closure
  expansion still runs and adds any new transitive refs.

### Testing

- **`closure.rs` unit tests** on hand-built `EdgeGraph` fixtures:
  - linear chain (A → B → C → fixed-point includes all three).
  - diamond closure (A → B, A → C, B → D, C → D — D appears once).
  - edge-kind filter (irrelevant kinds like `calls` ignored).
  - idempotence (closure-of-closure equals closure).
  - unregistered-repo edge target → structured error.
  - unknown-component edge target → structured error.

- **`mount_with_closure.rs` integration tests:**
  - two registered repos with a cross-repo `depends-on` edge in
    `related-components.yaml`. Mount a plan picking one component;
    assert both repos get worktrees and the right `targets.yaml` rows.
  - second invocation with the same input is a no-op.
  - third invocation with an additional ref appends only the new rows.

- **`apply_targets.rs` test:** extend the existing fake-validated
  fixture with a second registered repo and an inter-repo edge; assert
  closure-driven mounting through the migration path.

- **CLI verb test:** happy-path invocation through the
  `cli/error_envelope` boundary plus the unregistered-repo error path.

- **`mount_target` itself stays unchanged**, so its existing suite
  continues to cover the per-mount layer (idempotence,
  branch-conflict recovery, multi-component-shared-worktree).

## Risks and mitigations

- **Atlas index is the closure's source of truth.** A stale or
  incomplete `related-components.yaml` produces a stale closure. We
  surface this with actionable errors when an edge points at an
  unknown or unregistered component, but a *missing* edge produces
  silent under-mounting — the build will then fail the way it does
  today. Acceptable for v1; the LLM can use `add-target` to fix it.

- **Component-id ambiguity across repos.** The migration path already
  hardened the (repo, component) disambiguation. The closure walker
  reuses the same lookup; ambiguous edges produce a structured error
  rather than a silent wrong-mount.

- **Day-to-day source repo accumulates plan branches and worktrees.**
  Existing concern under `architecture-next.md`; closure makes it
  more frequent but not qualitatively different. Cleanup at finish is
  a separate concern.

## References

- `src/state/targets/mount.rs` — current per-component mount logic;
  unchanged by this design.
- `src/state/targets/schema.rs` — `Target` row shape; unchanged.
- `src/component_ref.rs` — `ComponentRef::from_str` parses the
  `<repo>:<component>` CLI form; reused by the new `add-target` verb.
- `src/atlas.rs:292+` — (repo_slug, component) iterator and
  `EdgeGraph`; reused by the closure walker.
- `src/migrate_v1_v2/apply_targets.rs` — current `mount_target` loop;
  replaced by one call to `mount_with_closure`.
- `docs/architecture-next.md` §Targets and worktrees, §Pure isolation
  against main — settled isolation policy that this design extends.
- Memory: "Harden migrate-targets against component-id ambiguity"
  (commit `aadff74`).
- Memory: "Take plan names, not directories, on `run` and `survey`"
  (commit `b98c3da`) — same convention applies to `plan add-target`.
