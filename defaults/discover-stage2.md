# Discovery — Stage 2: Infer Cross-Project Edges

You are given a collection of per-project interaction-surface records.
Your task is to propose relationship edges between catalogued projects
based on what their surfaces reveal.

## Edge kinds

- `sibling(A, B)` — peer-level relationship: two projects share a
  purpose, speak the same protocol, or exchange the same data format as
  peers. Order-insensitive.
- `parent-of(A, B)` — A produces artifacts, files, or contracts that B
  consumes. Order-sensitive: parent first. If A's `produces_files`
  matches B's `consumes_files`, or A serves an endpoint B calls, that
  is evidence for `parent-of(A, B)`.

## Matching signals

Propose edges when you see evidence such as:
- Overlapping file paths or globs between one project's `produces_files`
  and another's `consumes_files`.
- Matching network endpoints (server vs. client of the same protocol/
  address).
- Shared data format names (same struct / schema / message type).
- Shared external tools that suggest tight coupling (e.g., both projects
  spawn `some-custom-daemon`).
- Direct cross-project mentions in `explicit_cross_project_mentions`,
  *especially* when reciprocated by the other project.
- Semantic purpose overlap (both describe themselves as "task queue",
  "config loader", etc.) — use judgement here.

## Insufficient signals (do NOT propose edges from these alone)

These patterns are too weak to justify an edge on their own. Require
direct evidence from the matching-signals list above before proposing.

- **Shared upstream dependencies.** Two projects independently mentioning
  the same *third* catalog project in their `explicit_cross_project_mentions`
  is NOT evidence of a `sibling` or `parent-of` edge between those two
  projects. Many unrelated projects share infrastructure dependencies.
- **Same programming language or ecosystem.** Both being Rust crates,
  Racket packages, Swift apps, or Node packages is not a relationship.
- **Generic or trivial file-glob overlap.** Patterns like `*.txt`,
  `**/*.rkt`, `~/.config/**`, or any whole-language source-tree glob
  are too broad to constitute file-level coupling. Require a specific,
  named file or a narrow glob whose match set is plausibly produced by
  one project and consumed by another.
- **Same external tools alone.** Both projects spawning `git` or `bash`
  is not evidence; both spawning a *bespoke* binary owned by one of
  them is.

Edges should rest on direct evidence: A produces a specific artifact B
consumes, A serves an endpoint B calls, A and B implement the same named
specification, or one project explicitly names the other in its prose.
When in doubt, omit the edge — false positives are costlier than missed
edges since the user reviews proposals manually.

## Output format

Write YAML to `{{PROPOSALS_OUTPUT_PATH}}` matching this shape:

```yaml
generated_at: <ISO-8601 UTC timestamp>
proposals:
  - kind: sibling | parent-of
    participants: [<name>, <name>]    # parent first for parent-of
    rationale: |
      <one paragraph citing specific surface fields from the input>
    supporting_surface_fields:
      - <e.g., "Alpha.surface.produces_files">
      - <e.g., "Beta.surface.consumes_files">
```

Do NOT emit `schema_version` or `source_tree_shas` — those are injected
by the caller. Only propose edges between projects that appear in the
input. Only use project names exactly as they appear in the input —
no paths, no aliases.

After writing the YAML, your final message should confirm the path
written. No other output is required.

## Input

The input below lists every catalogued project's surface record.

---
{{SURFACE_RECORDS_YAML}}
