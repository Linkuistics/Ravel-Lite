//! Pure build-closure expansion for plan targets.
//!
//! Given an initial set of `(repo_slug, component_id)` refs, walk the
//! cross-repo `EdgeGraph` along *build/link* edges to a fixed point,
//! returning every component that must be mounted for the build to
//! succeed. No I/O — the caller supplies the `Catalog` and `EdgeGraph`.
//!
//! The build-edge filter is the directed kinds that imply "you can't
//! compile/link target A without target B's source on disk":
//! `depends-on`, `links-statically`, `links-dynamically`,
//! `has-optional-dependency`. Non-build kinds (`calls`, `invokes`,
//! `embeds`, `tests`, `provides-fixtures-for`, `conforms-to`,
//! `co-implements`, `communicates-with`, `describes`, etc.) are not
//! followed — they describe runtime/test/structural relations the LLM
//! brings in deliberately via the `add-target` CLI verb.
//!
//! Edge participants are bare component ids; host-repo resolution
//! goes through `Catalog::iter_components()`. Components whose ids
//! collide across repos produce a structured ambiguity error rather
//! than silent wrong-mounting (see memory: "Harden migrate-targets
//! against component-id ambiguity").
//!
//! See `docs/superpowers/specs/2026-05-05-multi-repo-plan-build-closure-design.md`.

use std::collections::{BTreeSet, VecDeque};

use anyhow::Result;
use component_ontology::EdgeKind;

use crate::atlas::{Catalog, EdgeGraph};
use crate::bail_with;
use crate::cli::ErrorCode;

/// Edge kinds that imply a build-time source-on-disk requirement. All
/// directed.
const BUILD_KINDS: &[EdgeKind] = &[
    EdgeKind::DependsOn,
    EdgeKind::LinksStatically,
    EdgeKind::LinksDynamically,
    EdgeKind::HasOptionalDependency,
];

/// Walk `graph` from `initial_refs` along [`BUILD_KINDS`] edges to a
/// fixed point. Returns the closed set, including the initial refs, in
/// BFS discovery order (initial set first, then transitive deps in the
/// order they were reached).
///
/// Errors when an edge target's component id matches no entry in the
/// catalog (stale edge after rename, or unindexed repo) or matches more
/// than one (ambiguous bare id across repos). Does not error on initial
/// refs whose components are absent from the catalog — that validation
/// is the orchestrator's job, since orchestrators may want to fail
/// earlier with their own message shape.
pub fn expand_build_closure(
    initial_refs: &[(String, String)],
    graph: &EdgeGraph,
    catalog: &Catalog,
) -> Result<Vec<(String, String)>> {
    let mut visited: BTreeSet<(String, String)> = BTreeSet::new();
    let mut order: Vec<(String, String)> = Vec::new();
    let mut queue: VecDeque<(String, String)> = VecDeque::new();

    for r in initial_refs {
        if visited.insert(r.clone()) {
            order.push(r.clone());
            queue.push_back(r.clone());
        }
    }

    while let Some((_repo, component_id)) = queue.pop_front() {
        for edge in &graph.edges {
            if !BUILD_KINDS.contains(&edge.kind) {
                continue;
            }
            // All BUILD_KINDS are directed; participants[0] is the
            // dependent, participants[1] is the dependency. Defensive
            // check guards against future ontology changes.
            if !edge.kind.is_directed() {
                continue;
            }
            if edge.participants[0] != component_id {
                continue;
            }
            let target_id = &edge.participants[1];
            let target_repo = resolve_host_repo(catalog, target_id, &component_id, edge.kind)?;
            let key = (target_repo, target_id.clone());
            if visited.insert(key.clone()) {
                order.push(key.clone());
                queue.push_back(key);
            }
        }
    }
    Ok(order)
}

fn resolve_host_repo(
    catalog: &Catalog,
    target_id: &str,
    source_id: &str,
    kind: EdgeKind,
) -> Result<String> {
    let mut hits: Vec<String> = Vec::new();
    for (slug, component) in catalog.iter_components() {
        if component.id == target_id {
            hits.push(slug.to_string());
        }
    }
    match hits.as_slice() {
        [] => bail_with!(
            ErrorCode::NotFound,
            "build closure: edge {source_id:?} -[{kind}]-> {target_id:?} targets a \
             component not present in any registered repo's `.atlas/components.yaml`. \
             Either an edge in `<repo>/.atlas/related-components.yaml` is stale (re-run \
             `atlas index <repo>`) or the target component was renamed.",
            kind = kind.as_str(),
        ),
        [single] => Ok(single.clone()),
        _ => bail_with!(
            ErrorCode::Conflict,
            "build closure: edge {source_id:?} -[{kind}]-> {target_id:?} is ambiguous; \
             component {target_id:?} appears in multiple registered repos: [{repos}]. \
             Disambiguate at the source — every component id must be unique across the \
             registered catalog.",
            kind = kind.as_str(),
            repos = hits.join(", "),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_index::{ComponentEntry, ComponentsFile};
    use component_ontology::{Edge, EdgeKind, LifecycleScope};
    use indexmap::IndexMap;
    use std::path::PathBuf;

    use crate::atlas::{EdgeGraph, RepoCatalog};

    fn comp(id: &str) -> ComponentEntry {
        let yaml = format!(
            "id: {id}\nkind: rust-library\nevidence_grade: strong\nrationale: test\n"
        );
        serde_yaml::from_str(&yaml).expect("valid ComponentEntry yaml")
    }

    fn repo_catalog(slug: &str, components: &[&str]) -> RepoCatalog {
        let file = ComponentsFile {
            schema_version: 1,
            root: PathBuf::from(format!("/fake/{slug}")),
            generated_at: "2026-05-05T00:00:00Z".into(),
            cache_fingerprints: Default::default(),
            components: components.iter().map(|c| comp(c)).collect(),
        };
        RepoCatalog {
            local_path: PathBuf::from(format!("/fake/{slug}")),
            components_yaml: PathBuf::from(format!("/fake/{slug}/.atlas/components.yaml")),
            file,
        }
    }

    fn catalog(repos: &[(&str, &[&str])]) -> Catalog {
        let mut map = IndexMap::new();
        for (slug, components) in repos {
            map.insert((*slug).to_string(), repo_catalog(slug, components));
        }
        Catalog {
            repos: map,
            freshness: Vec::new(),
        }
    }

    fn directed_edge(kind: EdgeKind, from: &str, to: &str) -> Edge {
        Edge {
            kind,
            lifecycle: LifecycleScope::Build,
            participants: vec![from.into(), to.into()],
            evidence_grade: component_ontology::EvidenceGrade::Strong,
            evidence_fields: vec!["test".into()],
            rationale: "test fixture".into(),
        }
    }

    fn graph(edges: Vec<Edge>) -> EdgeGraph {
        EdgeGraph { edges }
    }

    fn r(repo: &str, comp: &str) -> (String, String) {
        (repo.into(), comp.into())
    }

    #[test]
    fn empty_initial_set_returns_empty() {
        let cat = catalog(&[]);
        let g = graph(vec![]);
        let out = expand_build_closure(&[], &g, &cat).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn single_ref_no_outgoing_edges_returns_just_initial() {
        let cat = catalog(&[("solo", &["a"])]);
        let g = graph(vec![]);
        let out = expand_build_closure(&[r("solo", "a")], &g, &cat).unwrap();
        assert_eq!(out, vec![r("solo", "a")]);
    }

    #[test]
    fn linear_chain_in_same_repo_collects_full_chain() {
        // a -> b -> c, all in repo `r1`. Closure includes all three.
        let cat = catalog(&[("r1", &["a", "b", "c"])]);
        let g = graph(vec![
            directed_edge(EdgeKind::DependsOn, "a", "b"),
            directed_edge(EdgeKind::DependsOn, "b", "c"),
        ]);
        let out = expand_build_closure(&[r("r1", "a")], &g, &cat).unwrap();
        assert_eq!(out, vec![r("r1", "a"), r("r1", "b"), r("r1", "c")]);
    }

    #[test]
    fn diamond_dedups_shared_target() {
        // a -> b, a -> c, b -> d, c -> d. d must appear exactly once.
        let cat = catalog(&[("r1", &["a", "b", "c", "d"])]);
        let g = graph(vec![
            directed_edge(EdgeKind::DependsOn, "a", "b"),
            directed_edge(EdgeKind::DependsOn, "a", "c"),
            directed_edge(EdgeKind::DependsOn, "b", "d"),
            directed_edge(EdgeKind::DependsOn, "c", "d"),
        ]);
        let out = expand_build_closure(&[r("r1", "a")], &g, &cat).unwrap();
        let mut sorted = out.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec![r("r1", "a"), r("r1", "b"), r("r1", "c"), r("r1", "d")]
        );
        assert_eq!(out.len(), 4, "no duplicates");
    }

    #[test]
    fn cross_repo_dependency_pulls_in_sibling() {
        // a in r1 depends-on b in r2. Closure must include b under r2.
        let cat = catalog(&[("r1", &["a"]), ("r2", &["b"])]);
        let g = graph(vec![directed_edge(EdgeKind::DependsOn, "a", "b")]);
        let out = expand_build_closure(&[r("r1", "a")], &g, &cat).unwrap();
        assert_eq!(out, vec![r("r1", "a"), r("r2", "b")]);
    }

    #[test]
    fn non_build_edge_kinds_are_ignored() {
        // a calls b — `calls` is not in BUILD_KINDS, so b must not be
        // pulled in. The LLM uses `add-target` for runtime references.
        let cat = catalog(&[("r1", &["a", "b"])]);
        let g = graph(vec![directed_edge(EdgeKind::Calls, "a", "b")]);
        let out = expand_build_closure(&[r("r1", "a")], &g, &cat).unwrap();
        assert_eq!(out, vec![r("r1", "a")]);
    }

    #[test]
    fn all_four_build_kinds_are_followed() {
        // One distinct BUILD_KIND per leaf component, ensuring the
        // closure walker honours every kind in the set.
        let cat = catalog(&[("r1", &["root", "dep", "stat", "dyn", "opt"])]);
        let g = graph(vec![
            directed_edge(EdgeKind::DependsOn, "root", "dep"),
            directed_edge(EdgeKind::LinksStatically, "root", "stat"),
            directed_edge(EdgeKind::LinksDynamically, "root", "dyn"),
            directed_edge(EdgeKind::HasOptionalDependency, "root", "opt"),
        ]);
        let mut out = expand_build_closure(&[r("r1", "root")], &g, &cat).unwrap();
        out.sort();
        assert_eq!(
            out,
            vec![
                r("r1", "dep"),
                r("r1", "dyn"),
                r("r1", "opt"),
                r("r1", "root"),
                r("r1", "stat"),
            ]
        );
    }

    #[test]
    fn cycle_terminates_without_duplicates() {
        // a -> b -> a. Closure must terminate, both appear once.
        let cat = catalog(&[("r1", &["a", "b"])]);
        let g = graph(vec![
            directed_edge(EdgeKind::DependsOn, "a", "b"),
            directed_edge(EdgeKind::DependsOn, "b", "a"),
        ]);
        let out = expand_build_closure(&[r("r1", "a")], &g, &cat).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&r("r1", "a")));
        assert!(out.contains(&r("r1", "b")));
    }

    #[test]
    fn closure_of_closure_is_idempotent() {
        // Running the walker on a previously closed set adds nothing.
        let cat = catalog(&[("r1", &["a", "b", "c"])]);
        let g = graph(vec![
            directed_edge(EdgeKind::DependsOn, "a", "b"),
            directed_edge(EdgeKind::DependsOn, "b", "c"),
        ]);
        let first = expand_build_closure(&[r("r1", "a")], &g, &cat).unwrap();
        let second = expand_build_closure(&first, &g, &cat).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn unknown_target_component_errors_with_actionable_message() {
        // a depends-on x, but x is not in any registered repo's index.
        let cat = catalog(&[("r1", &["a"])]);
        let g = graph(vec![directed_edge(EdgeKind::DependsOn, "a", "x")]);
        let err = expand_build_closure(&[r("r1", "a")], &g, &cat).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("\"x\""), "must name the missing target: {msg}");
        assert!(msg.contains("atlas index"), "must suggest atlas re-index: {msg}");
    }

    #[test]
    fn ambiguous_target_component_errors_with_repo_list() {
        // a depends-on b; b exists in both r2 and r3. The walker must
        // refuse to guess and surface both candidates.
        let cat = catalog(&[("r1", &["a"]), ("r2", &["b"]), ("r3", &["b"])]);
        let g = graph(vec![directed_edge(EdgeKind::DependsOn, "a", "b")]);
        let err = expand_build_closure(&[r("r1", "a")], &g, &cat).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ambiguous"), "must say ambiguous: {msg}");
        assert!(msg.contains("r2"), "must list candidate r2: {msg}");
        assert!(msg.contains("r3"), "must list candidate r3: {msg}");
    }

    #[test]
    fn initial_set_with_both_endpoints_does_not_duplicate() {
        // Initial set already contains both ends of an edge; the
        // outgoing edge from the first must not re-add the second.
        let cat = catalog(&[("r1", &["a", "b"])]);
        let g = graph(vec![directed_edge(EdgeKind::DependsOn, "a", "b")]);
        let out = expand_build_closure(&[r("r1", "a"), r("r1", "b")], &g, &cat).unwrap();
        assert_eq!(out, vec![r("r1", "a"), r("r1", "b")]);
    }
}
