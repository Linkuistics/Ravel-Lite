# Component Relationship Ontology — Reference

**Status:** Reference specification.
**Applies to:** `related-components.yaml`, the `ravel-lite state
related-components` CLI, the Stage 2 discover prompt, and the Rust
library that implements the schema (`component-ontology`).

## 1. Purpose

This document is the canonical specification of the component-relationship
ontology. Every implementation artifact derives from it:

- The Rust type enum (`EdgeKind`, `LifecycleScope`, `EvidenceGrade`)
  matches §5 exactly.
- The discover Stage 2 prompt's edge-kind vocabulary matches §5 exactly.
- The `defaults/ontology.yaml` file shipped with Ravel-Lite matches §5
  exactly.

Divergence between this document and any implementation is a bug in the
implementation.

## 2. Components as the unit of relationship

The ontology describes edges between **components**. A *component* is any
addressable unit of software whose relationships to other components are
worth cataloguing.

### 2.1 What counts as a component

The ontology is deliberately unit-agnostic. Concrete examples:

- A whole *project* — a Cargo workspace, a git repository, a Node
  package.
- A *crate* within a workspace — when intra-workspace coupling matters.
- A *service* in a multi-service deployment.
- A *subsystem* or bounded module within a larger project.
- An *external specification* (an RFC, a wire-protocol spec document) —
  components on either side of a `conforms-to` edge must both be
  catalogued, and the spec is one of them.
- A *third-party library* referenced by the catalog, when relationships
  to it are worth recording.

The ontology requires only that each component have a stable,
catalog-scoped identifier. It does not require that components share a
language, repository, runtime, or ownership.

### 2.2 Identifier scheme

A component identifier is an opaque string, unique within the catalog the
consumer supplies. The catalog schema is a consumer concern — a catalog
of whole projects uses project names; a mixed-scope catalog could use
prefixed identifiers (e.g., `service:foo`, `crate:bar`) without schema
changes here; only the identifier format becomes richer.

The library treats identifiers as opaque: equality, ordering, and display
only. Identifier validation (existence, shape) is the catalog's
responsibility, not the ontology's.

## 3. The model: three orthogonal axes

Every edge is a tuple `(kind, lifecycle, direction)` over two component
identifiers, annotated with evidence.

### 3.1 Axis 1 — `kind` (what is coupled)

Seven thematic families; 17 total kinds defined in §5.

| Family | What flows across the seam |
|---|---|
| Dependency | Transitive reachability: A needs B to function |
| Linkage | Compile/link-time symbol resolution |
| Generation | One side authors the other side's source artifacts |
| Communication | Live messages at runtime (IPC / network) |
| Orchestration | One side drives the other's lifecycle |
| Testing | One side exercises the other |
| Specification | One side defines contracts the other conforms to |

### 3.2 Axis 2 — `lifecycle` (when the coupling is active)

Seven scopes. An edge declares exactly one; multiple scopes for one pair
become multiple edges (§3.5).

| Scope | Active during | Example |
|---|---|---|
| `design` | Human authoring, shared specs | Two components implementing the same RFC |
| `codegen` | Source generation from another source | Protobuf emits structs; a schema-emitter produces YAML definitions |
| `build` | Compilation, packaging | Library dep resolved at `cargo build` |
| `test` | Test execution | Test fixtures, mocks, integration harness |
| `deploy` | Install / provisioning | Container image, binary packaging |
| `runtime` | Live execution | RPC, shared memory, file-watch IPC |
| `dev-workflow` | Developer loop, not shipped | A tool that spawns the component under dev |

Notes:

- `design` edges are the weakest by construction — they capture "two
  independent implementations of the same spec" with no artifact flow.
- `codegen` produces *source* (committed, edited, regenerated); `build`
  consumes source to produce artifacts. The distinction matters: a tool
  that emits committed YAML/Rust for another component is `codegen`, not
  `build`.

### 3.3 Axis 3 — `direction` (who-on-whom)

Direction is a property of the **kind**, not a free field:

- **Directed** kinds are order-sensitive. Canonical order is fixed per
  kind (§6).
- **Symmetric** kinds are order-insensitive. Canonicalised by sorting
  identifiers.

Fixing direction per kind means every kind has type-system-enforced
semantics for participant order — there is no free-form "parent first"
convention that has to be documented verbally and checked manually.

### 3.4 Evidence and grade

Every edge carries:

- `evidence_grade: strong | medium | weak`
- `evidence_fields: [<surface-field-reference>, …]` — Stage 1 surface
  paths the edge is grounded in (e.g., `Ravel-Lite.produces_files`,
  `Ravel.consumes_files`).
- `rationale` — free-form prose.

Grade heuristics:

- **strong** — symmetric artifact match (A produces X, B consumes X); a
  named wire protocol both sides declare; a reciprocated explicit
  mention.
- **medium** — one-sided evidence, shared format name without location,
  or a shared external tool that's clearly one component's bespoke
  binary.
- **weak** — prose overlap, purpose similarity, shared data-format name
  without location. Weak edges are permitted but must declare weakness.

### 3.5 Multiplicity

A pair of components may have multiple edges with distinct
`(kind, lifecycle)` tuples. This is normal: one component may both
`generates@codegen` schemas consumed by another **and**
`orchestrates@dev-workflow` that same component's agent loop. Two edges,
two kinds, two scopes, one pair.

Dedup key: `(kind, lifecycle, canonical-participants)` (see §7.3).

## 4. Prior art alignment

The ontology explicitly aligns with, adopts, or departs from the
following bodies of work:

- **SPDX 3.0.1 `RelationshipType` + `LifecycleScopeType`.** Closest fit.
  We adopt the **kind × lifecycle factoring** and align kind names
  where the concept matches (correspondence column in §5). We do **not**
  adopt SPDX wholesale — its vulnerability, licensing, and bom-ref
  elements are SBOM concerns orthogonal to cross-component coupling.
- **Stevens/Myers/Constantine structured-design coupling** (1974).
  Classifies *intra-program* module coupling. Informs the surface-based
  framing (an edge is characterised by what crosses the seam) but does
  not contribute kind names — its units are functions, not components.
- **Maven scopes / Gradle configurations.** Inform the lifecycle-scope
  enum (`compile`, `runtime`, `test`, `provided-by-host`). They aren't
  edge kinds — they're lifecycle qualifiers on a single kind
  (`depends-on`).
- **CycloneDX component scope** (`required | optional | excluded`). A
  per-edge modality bit. We capture the required/optional distinction
  via the dedicated `has-optional-dependency` vs. `depends-on` kind
  pair rather than a separate field.
- **Bazel/Pants/Nx.** Reinforce that **codegen is a first-class edge**
  (Bazel's `genrule`). Maps directly to the `generates` kind.

## 5. Edge-kind reference

Kind names are kebab-case. Each entry: direction · typical lifecycle(s) ·
definition · SPDX alignment · primary Stage 1 evidence.

### 5.1 Dependency family

- **`depends-on`** · directed · `build` | `runtime`
  A requires B to function at the declared scope. Library-level
  dependency with a direction.
  SPDX: `dependsOn`.
  Evidence: package-manifest entries, import statements, `consumes_files`
  referencing B's manifest.

- **`has-optional-dependency`** · directed · `build` | `runtime`
  A can function without B but gains capability when B is present.
  SPDX: `hasOptionalDependency`.
  Evidence: `optional-dependencies` manifest sections, feature flags,
  plugin discovery.

- **`provided-by-host`** · directed · `runtime`
  A expects B to be present in the execution environment, not bundled.
  SPDX: `hasProvidedDependency`.
  Evidence: "expects X in PATH", servlet-style container-provided
  comments.

### 5.2 Linkage family

- **`links-statically`** · directed · `build`
  A embeds B's compiled code in its own artifact.
  SPDX: `hasStaticLink`.
  Evidence: static-lib dep in build manifest.

- **`links-dynamically`** · directed · `runtime`
  A loads B at runtime (shared object, dylib, plugin).
  SPDX: `hasDynamicLink`.
  Evidence: `dlopen` calls, dynamic-lib manifest entries, plugin-loader
  config.

### 5.3 Generation family

- **`generates`** · directed · `codegen`
  A's tooling emits source that is committed to B (or to a location B
  consumes as source).
  SPDX: `generates`.
  Evidence: A's `produces_files` matches B's source tree; B documents
  "run A to regenerate".

- **`scaffolds`** · directed · `dev-workflow`
  A emits a one-shot initial structure for B that is not regenerated on
  change. The coupling ends at B's first commit.
  SPDX: (no direct equivalent — closest is `generates` with explicit
  `noLifecycleScope`).
  Evidence: `create-X` templates, cookiecutter-style tools.

### 5.4 Communication family

- **`communicates-with`** · symmetric · `runtime`
  A and B exchange messages at runtime over a named transport, as peers.
  Use when no clear client/server split exists.
  SPDX: no direct equivalent.
  Evidence: overlapping `network_endpoints` with matching protocol;
  shared `data_formats` that both emit and consume.

- **`calls`** · directed · `runtime`
  A is the client of an endpoint B serves.
  SPDX: no direct equivalent (nearest: `usesTool`).
  Evidence: A's `network_endpoints` contains an address B's
  `network_endpoints` serves.

### 5.5 Orchestration family

- **`invokes`** · directed · `dev-workflow` | `runtime`
  A spawns B as a subprocess. Distinguish lifecycle: one-shot CLI
  invocation is `dev-workflow`; persistent process management is
  `runtime`.
  SPDX: `invokedBy` (inverse).
  Evidence: A's `external_tools_spawned` names B's binary; B exports that
  binary as its primary artifact.

- **`orchestrates`** · directed · `dev-workflow` | `runtime`
  Stronger than `invokes`: A manages B's lifecycle, state, and
  multi-step workflow.
  SPDX: no direct equivalent.
  Evidence: A's prose documents driving B through phases; A reads/writes
  B's state files; reciprocated explicit mentions.

- **`embeds`** · directed · `runtime`
  A runs B in-process (library embedding, WASM, subprocess-in-pipe).
  Distinct from `links-dynamically`: B is a whole program, not a
  library.
  SPDX: no direct equivalent.
  Evidence: A documents embedding B's runtime.

### 5.6 Testing family

- **`tests`** · directed · `test`
  A is a test harness for B (A's primary purpose is to exercise B).
  SPDX: `hasTest` (inverse).
  Evidence: A's purpose prose; A's `consumes_files` includes B's source.

- **`provides-fixtures-for`** · directed · `test`
  A provides test data, mocks, or fixtures that B's test suite loads.
  SPDX: no direct equivalent (related: `hasInput` at `test` scope).
  Evidence: fixture file paths overlap; prose.

### 5.7 Specification family

- **`conforms-to`** · directed · `design`
  A implements a spec defined in B (protocol, schema, RFC-internal).
  SPDX: `hasSpecification` (inverse).
  Evidence: B's primary artifact is a spec document; A references it.

- **`co-implements`** · symmetric · `design`
  A and B are parallel implementations of the same external spec that
  neither component owns (two LSP clients; two MCP servers).
  SPDX: no direct equivalent (distantly: `hasVariant`).
  Evidence: both components declare implementing the same named spec;
  no artifact flows between them.

- **`describes`** · directed · `design`
  A documents B (docs repo, architecture notes, external user guide).
  SPDX: `describes` / `hasDocumentation`.
  Evidence: A's purpose is documentation; A's name or contents reference
  B.

### 5.8 Out of scope for this ontology

- **`shares-types`** — reducible to `depends-on` (A imports B's type
  defs) or `generates` (B's codegen emits A's types).
- **Negative edges** ("A and B are not related") — deferred (§10).
- **Numeric confidence scores** — three evidence grades are enough for
  review-gate workflow.
- **Hyperedges** (3+ participants) — binary-edge invariant retained
  (§10).

## 6. Direction and symmetry reference table

| Kind | Directed? | Canonical order |
|---|---|---|
| `depends-on` | yes | dependent first |
| `has-optional-dependency` | yes | dependent first |
| `provided-by-host` | yes | dependent first |
| `links-statically` | yes | binary first, lib second |
| `links-dynamically` | yes | loader first, loaded second |
| `generates` | yes | generator first, generated second |
| `scaffolds` | yes | scaffolder first |
| `communicates-with` | **no** | sorted |
| `calls` | yes | client first, server second |
| `invokes` | yes | parent process first |
| `orchestrates` | yes | orchestrator first |
| `embeds` | yes | host first, embedded second |
| `tests` | yes | tester first, tested second |
| `provides-fixtures-for` | yes | provider first |
| `conforms-to` | yes | implementer first, spec second |
| `co-implements` | **no** | sorted |
| `describes` | yes | describer first, described second |

## 7. On-disk schema

### 7.1 File

Path: `<config-root>/related-components.yaml`.

```yaml
schema_version: 2
edges:
  - kind: generates
    lifecycle: codegen
    participants: [Ravel-Lite, Ravel]
    evidence_grade: strong
    evidence_fields:
      - Ravel-Lite.produces_files
      - Ravel.consumes_files
    rationale: |
      Ravel-Lite emits LLM_STATE/<plan>/backlog.yaml schemas that Ravel's
      runtime reads; the schema definition lives in Ravel-Lite.

  - kind: orchestrates
    lifecycle: dev-workflow
    participants: [Ravel-Lite, Ravel]
    evidence_grade: strong
    evidence_fields:
      - Ravel-Lite.external_tools_spawned
      - Ravel-Lite.purpose
    rationale: |
      Ravel-Lite spawns claude / pi agent subprocesses as part of a phase
      loop it drives; it is Ravel's dev-workflow orchestrator.

  - kind: co-implements
    lifecycle: design
    participants: [ClientA, ClientB]         # symmetric: sorted
    evidence_grade: medium
    evidence_fields:
      - ClientA.purpose
      - ClientB.purpose
    rationale: |
      Both components implement the MCP stdio spec; neither owns the
      spec.
```

### 7.2 Field specification

- `schema_version: 2` — integer, required, exact match.
- `edges` — list of edge records.
  - `kind` — one of the kebab-case kinds in §5. Required.
  - `lifecycle` — one of the scopes in §3.2. Required.
  - `participants` — list of exactly two component identifiers.
    Distinct. For directed kinds, ordered per §6. For symmetric kinds,
    sorted.
  - `evidence_grade` — `strong | medium | weak`. Required.
  - `evidence_fields` — list of `<component>.<surface-field>` strings.
    May be empty only when `evidence_grade = weak` and `rationale`
    justifies it explicitly. Non-empty otherwise.
  - `rationale` — free-form prose. Required, non-empty.

### 7.3 Dedup / canonical key

```
key(edge) = (edge.kind, edge.lifecycle, participants′)
where participants′ = sorted(edge.participants)  if edge.kind is symmetric
                    = edge.participants          otherwise
```

Two edges with equal `key` are duplicates. Idempotent inserts (same key)
are no-ops. Distinct keys on the same participant pair are legal and
expected (§3.5).

### 7.4 Conflict detection

One check: **same directed kind, reversed participants** (e.g., both
`depends-on(A, B)` and `depends-on(B, A)` at the same lifecycle) is a
modelling error and is rejected. There is no cross-kind conflict —
multiple kinds per pair are expected.

## 8. The ontology YAML — `defaults/ontology.yaml`

A single language-neutral file ships with Ravel-Lite at
`defaults/ontology.yaml`. It is the data form of §5 + §6 + §3.2. Its
purpose is twofold:

1. **Single source of truth.** A build-time test asserts that the
   `EdgeKind` Rust enum and the YAML list agree exactly. Adding a kind
   in one place without the other fails the test.
2. **Prompt input.** `defaults/discover-stage2.md` substitutes the kind
   list into the prompt via a token (`{{ONTOLOGY_KINDS}}`) rather than
   hard-coding it in prose. Vocabulary evolves in one place.

Sketch:

```yaml
schema_version: 1   # of the ontology file itself; independent of the
                    # related-components.yaml schema_version
kinds:
  - name: depends-on
    family: dependency
    directed: true
    lifecycles: [build, runtime]
    spdx: dependsOn
    description: |
      A requires B to function at the declared scope…

  - name: co-implements
    family: specification
    directed: false
    lifecycles: [design]
    spdx: null
    description: |
      …

lifecycles:
  - name: design
    description: Human authoring, shared specs
  - name: codegen
    description: Source generation from another source
  # …

evidence_grades:
  - name: strong
    criterion: …
  # …
```

Consumers outside Ravel-Lite can parse this file without pulling in the
Rust crate.

## 9. Rust library surface

The library is `component-ontology`. Inside Ravel-Lite it lives at
`src/ontology/` and is re-exported via the crate root; when it graduates
to a workspace member crate the path becomes
`crates/component-ontology/`.

### 9.1 In scope for the library

- Types: `EdgeKind`, `LifecycleScope`, `EvidenceGrade`, `Edge`,
  `RelatedComponentsFile`.
- `serde`-driven load / save with `schema_version` gate; atomic write
  helper.
- `Edge::canonical_key`, `Edge::is_directed`, `Edge::validate`.
- `RelatedComponentsFile::add_edge` with idempotent dedup.
- `rename_component_in_edges(&mut self, old, new)`.
- Hard-error loader for non-matching `schema_version` values (no
  in-memory upgrade path).
- `SCHEMA_VERSION: u32 = 2` constant.
- An optional `validate_against_ontology(ontology: &OntologyYaml)`
  helper, for callers that want drift detection between their in-code
  enum and `ontology.yaml` (§8).

### 9.2 Out of scope for the library

- Catalog integration (resolver between component identifiers and
  filesystem or repository state). The library treats identifiers as
  opaque strings.
- The discover pipeline (Stage 1 / Stage 2). These live in Ravel-Lite;
  the library provides the types they serialise into.
- The CLI wrapper. `ravel-lite state related-components …` is a thin
  adapter in Ravel-Lite.
- Prompt templates. The Stage 2 prompt is Ravel-Lite's property; the
  library may expose kind-name constants that prompts substitute in,
  but the prompt itself is not shipped by the library.
- Filesystem locations. Callers supply a path. The library has no
  opinion on `<config-root>`.

### 9.3 Dependency posture

Minimal. The library depends on `serde`, `serde_yaml`, `anyhow`,
`thiserror`. No tokio, no clap, no filesystem conventions.

### 9.4 Public API sketch

```rust
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeKind {
    DependsOn, HasOptionalDependency, ProvidedByHost,
    LinksStatically, LinksDynamically,
    Generates, Scaffolds,
    CommunicatesWith, Calls,
    Invokes, Orchestrates, Embeds,
    Tests, ProvidesFixturesFor,
    ConformsTo, CoImplements, Describes,
}

impl EdgeKind {
    pub fn is_directed(self) -> bool { /* §6 */ }
    pub fn as_str(self) -> &'static str { /* kebab */ }
    pub fn parse(s: &str) -> Option<Self> { /* … */ }
    pub fn all() -> &'static [EdgeKind] { /* iteration */ }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LifecycleScope {
    Design, Codegen, Build, Test, Deploy, Runtime, DevWorkflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceGrade { Strong, Medium, Weak }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub lifecycle: LifecycleScope,
    pub participants: Vec<String>,
    pub evidence_grade: EvidenceGrade,
    #[serde(default)]
    pub evidence_fields: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedComponentsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub edges: Vec<Edge>,
}
```

### 9.5 Invariants enforced by the library

- `schema_version == 2` on both read and write. Any other version is a
  hard error; the library does not attempt in-memory upgrade.
- `participants.len() == 2`, `participants[0] != participants[1]`.
- For directed kinds: participants stored in semantic order (§6).
- For symmetric kinds: participants stored sorted.
- `evidence_grade` present; `evidence_fields` non-empty unless
  `evidence_grade == Weak`.
- `rationale` non-empty.

### 9.6 Design principles

- **No host-application concepts leak.** No references to Ravel-Lite's
  phases, plans, backlog, subagents, or state directories. The library's
  universe is edges, kinds, lifecycles, components, evidence.
- **Narrow dependency footprint.** Adding a new dependency requires a
  documented reason.
- **Semver from day one.** The API stabilises before a 1.0 cut.
- **Portability-first naming.** File paths, config keys, prompt tokens
  — none of them appear in the crate. They live in the host
  application's thin adapter code.

## 10. Open questions

1. **Hyperedges.** `orchestrates(A, {B, C, D})` is more accurate for an
   orchestrator with multiple subjects than three binary edges.
   Deferred; revisit when real examples accumulate.
2. **Temporal decay.** Edges valid at a past snapshot but no longer.
   Stage 1 already caches on tree SHA; a per-edge
   `first_seen / last_confirmed` pair could let edges age gracefully.
   Deferred.
3. **Per-kind evidence schemas.** Typed discriminated-union per kind
   (`generates` requires `produces_files ↔ consumes_files`; `calls`
   requires an endpoint match) would catch mislabelled evidence at
   validation time. Deferred until a second tool consumes the graph.
4. **Negative edges.** "A and B look related but are not" — suppresses
   repeated false-positive proposals. Could be a separate
   `excluded_edges` list.
5. **Catalog pluralism.** If the catalog ever gains non-project
   components (crates, services), component identifiers will need a
   shape beyond whole-project names. The ontology itself does not
   change; the catalog schema does.
