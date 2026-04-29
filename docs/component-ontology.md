# Component-Ontology

Moved upstream. The component-relationship ontology
(`EdgeKind`, `LifecycleScope`, `EvidenceGrade`, the
`related-components.yaml` schema, edge-construction rules) now lives in
the `component-ontology` crate of the
[atlas-contracts](https://github.com/linkuistics/atlas-contracts)
workspace.

Canonical references:

- Crate README:
  [`crates/component-ontology/README.md`](https://github.com/linkuistics/atlas-contracts/blob/main/crates/component-ontology/README.md)
- Edge-kind YAML (single source of truth):
  [`defaults/ontology.yaml`](https://github.com/linkuistics/atlas-contracts/blob/main/defaults/ontology.yaml)

The host-side adapter — filename conventions, `<config-root>` joins,
the per-user `projects.yaml` resolver, and the `ravel-lite state
related-components` CLI verbs — lives in `src/related_components.rs`.
