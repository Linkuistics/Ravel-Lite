//! Generic directed-graph adjacency-list, the substrate for graph
//! algorithms (path-finding, strongly connected components) layered on
//! top of `atlas`'s component catalog.
//!
//! Kept independent of `atlas` so it stays algorithmically pure and
//! reusable: the graph speaks `N`, not `Component`. The
//! catalog-to-graph projection (which directed edges to include, how to
//! resolve component identifiers) lives in `atlas::build_directed_component_graph`.
//!
//! The store is two adjacency `BTreeMap`s keyed on the node — one for
//! out-edges, one for in-edges — plus a `BTreeSet` of every known node
//! (so isolated nodes survive enumeration). `BTree*` containers give
//! deterministic iteration order, which keeps test fixtures and
//! algorithm output stable without callers having to sort.
//!
//! `add_edge` is idempotent: inserting the same `(from, to)` twice
//! leaves the graph unchanged. This keeps adjacency lists clean so
//! BFS/DFS visit counts in downstream algorithms reflect graph
//! topology, not insertion redundancy.

use std::collections::{BTreeMap, BTreeSet};

/// Directed graph stored as adjacency lists. Generic over the node
/// identifier `N`; `atlas` instantiates it as `DirectedGraph<String>`
/// over bare component IDs.
#[derive(Debug, Clone)]
pub struct DirectedGraph<N: Ord + Clone> {
    nodes: BTreeSet<N>,
    out_edges: BTreeMap<N, Vec<N>>,
    in_edges: BTreeMap<N, Vec<N>>,
}

impl<N: Ord + Clone> Default for DirectedGraph<N> {
    fn default() -> Self {
        DirectedGraph {
            nodes: BTreeSet::new(),
            out_edges: BTreeMap::new(),
            in_edges: BTreeMap::new(),
        }
    }
}

impl<N: Ord + Clone> DirectedGraph<N> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `node` as a known node, even if it has no incident
    /// edges. Idempotent. This is what lets isolated catalog
    /// components surface in `nodes()`.
    pub fn add_node(&mut self, node: N) {
        self.nodes.insert(node);
    }

    /// Insert a directed edge `from → to`. Both endpoints become known
    /// nodes. Idempotent: a duplicate `(from, to)` is silently dropped
    /// so adjacency lists stay free of repeats.
    pub fn add_edge(&mut self, from: N, to: N) {
        self.nodes.insert(from.clone());
        self.nodes.insert(to.clone());
        let out = self.out_edges.entry(from.clone()).or_default();
        if !out.iter().any(|n| n == &to) {
            out.push(to.clone());
        }
        let inc = self.in_edges.entry(to).or_default();
        if !inc.iter().any(|n| n == &from) {
            inc.push(from);
        }
    }

    /// Outgoing neighbors of `node` in insertion order. Empty slice if
    /// `node` is unknown or has no out-edges.
    pub fn neighbors(&self, node: &N) -> &[N] {
        self.out_edges.get(node).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Incoming neighbors of `node` in insertion order. Empty slice if
    /// `node` is unknown or has no in-edges.
    pub fn reverse_neighbors(&self, node: &N) -> &[N] {
        self.in_edges.get(node).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every known node, in sorted order. Includes isolated nodes
    /// added via `add_node` that participate in no edge.
    pub fn nodes(&self) -> impl Iterator<Item = &N> {
        self.nodes.iter()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.out_edges.values().map(Vec::len).sum()
    }

    pub fn contains_node(&self, node: &N) -> bool {
        self.nodes.contains(node)
    }

    pub fn contains_edge(&self, from: &N, to: &N) -> bool {
        self.out_edges
            .get(from)
            .is_some_and(|outs| outs.iter().any(|n| n == to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_of_strs(edges: &[(&str, &str)]) -> DirectedGraph<String> {
        let mut g = DirectedGraph::new();
        for (from, to) in edges {
            g.add_edge((*from).to_string(), (*to).to_string());
        }
        g
    }

    #[test]
    fn new_graph_has_no_nodes_or_edges() {
        let g: DirectedGraph<String> = DirectedGraph::new();
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.edge_count(), 0);
        assert!(g.nodes().next().is_none());
    }

    #[test]
    fn add_edge_registers_both_endpoints_as_nodes() {
        let g = graph_of_strs(&[("A", "B")]);
        assert_eq!(g.node_count(), 2);
        assert!(g.contains_node(&"A".to_string()));
        assert!(g.contains_node(&"B".to_string()));
    }

    #[test]
    fn neighbors_returns_outgoing_targets_only() {
        let g = graph_of_strs(&[("A", "B"), ("A", "C"), ("B", "C")]);
        assert_eq!(g.neighbors(&"A".to_string()), ["B".to_string(), "C".to_string()]);
        assert_eq!(g.neighbors(&"B".to_string()), ["C".to_string()]);
        assert!(g.neighbors(&"C".to_string()).is_empty());
    }

    #[test]
    fn reverse_neighbors_returns_incoming_sources_only() {
        let g = graph_of_strs(&[("A", "C"), ("B", "C"), ("A", "B")]);
        assert_eq!(
            g.reverse_neighbors(&"C".to_string()),
            ["A".to_string(), "B".to_string()]
        );
        assert_eq!(g.reverse_neighbors(&"B".to_string()), ["A".to_string()]);
        assert!(g.reverse_neighbors(&"A".to_string()).is_empty());
    }

    #[test]
    fn directed_edge_direction_is_preserved() {
        // The hallmark assertion of a *directed* graph: A → B means A
        // is in B's reverse-neighbors but B is not in A's reverse-
        // neighbors, and likewise B is in A's out-neighbors but A is
        // not in B's out-neighbors.
        let g = graph_of_strs(&[("A", "B")]);
        assert!(g.contains_edge(&"A".to_string(), &"B".to_string()));
        assert!(!g.contains_edge(&"B".to_string(), &"A".to_string()));
        assert_eq!(g.neighbors(&"A".to_string()), ["B".to_string()]);
        assert!(g.neighbors(&"B".to_string()).is_empty());
        assert_eq!(g.reverse_neighbors(&"B".to_string()), ["A".to_string()]);
        assert!(g.reverse_neighbors(&"A".to_string()).is_empty());
    }

    #[test]
    fn add_edge_is_idempotent() {
        let mut g = DirectedGraph::new();
        g.add_edge("A".to_string(), "B".to_string());
        g.add_edge("A".to_string(), "B".to_string());
        g.add_edge("A".to_string(), "B".to_string());
        assert_eq!(g.edge_count(), 1);
        assert_eq!(g.neighbors(&"A".to_string()), ["B".to_string()]);
        assert_eq!(g.reverse_neighbors(&"B".to_string()), ["A".to_string()]);
    }

    #[test]
    fn add_node_registers_isolated_nodes() {
        let mut g: DirectedGraph<String> = DirectedGraph::new();
        g.add_node("solo".to_string());
        assert_eq!(g.node_count(), 1);
        assert!(g.contains_node(&"solo".to_string()));
        assert!(g.neighbors(&"solo".to_string()).is_empty());
        assert!(g.reverse_neighbors(&"solo".to_string()).is_empty());
    }

    #[test]
    fn add_node_is_idempotent() {
        let mut g: DirectedGraph<String> = DirectedGraph::new();
        g.add_node("solo".to_string());
        g.add_node("solo".to_string());
        assert_eq!(g.node_count(), 1);
    }

    #[test]
    fn nodes_iter_is_sorted_and_includes_isolated() {
        let mut g = graph_of_strs(&[("zeta", "alpha"), ("mu", "beta")]);
        g.add_node("isolated".to_string());
        let collected: Vec<&String> = g.nodes().collect();
        let names: Vec<&str> = collected.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta", "isolated", "mu", "zeta"]);
    }

    #[test]
    fn neighbors_returns_empty_slice_for_unknown_node() {
        let g = graph_of_strs(&[("A", "B")]);
        assert!(g.neighbors(&"unknown".to_string()).is_empty());
        assert!(g.reverse_neighbors(&"unknown".to_string()).is_empty());
    }

    #[test]
    fn cycles_are_supported_and_self_consistent() {
        // A → B → C → A: every node has exactly one out and one in.
        let g = graph_of_strs(&[("A", "B"), ("B", "C"), ("C", "A")]);
        assert_eq!(g.node_count(), 3);
        assert_eq!(g.edge_count(), 3);
        for node in ["A", "B", "C"] {
            assert_eq!(g.neighbors(&node.to_string()).len(), 1);
            assert_eq!(g.reverse_neighbors(&node.to_string()).len(), 1);
        }
    }

    #[test]
    fn opposite_directed_edges_between_two_nodes_coexist() {
        // A → B and B → A are distinct edges (a 2-cycle), not duplicates.
        let g = graph_of_strs(&[("A", "B"), ("B", "A")]);
        assert_eq!(g.edge_count(), 2);
        assert!(g.contains_edge(&"A".to_string(), &"B".to_string()));
        assert!(g.contains_edge(&"B".to_string(), &"A".to_string()));
    }

    #[test]
    fn graph_is_generic_over_arbitrary_ord_clone_node_types() {
        // Smoke test the generic bound with an integer node type, the
        // simplest non-String witness.
        let mut g: DirectedGraph<u32> = DirectedGraph::new();
        g.add_edge(1, 2);
        g.add_edge(2, 3);
        g.add_node(99);
        assert_eq!(g.node_count(), 4);
        assert_eq!(g.neighbors(&1), [2]);
        assert_eq!(g.reverse_neighbors(&3), [2]);
    }
}
