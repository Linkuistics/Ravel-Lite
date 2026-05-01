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

use std::collections::{BTreeMap, BTreeSet, VecDeque};

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

    /// BFS shortest path from `from` to `to`, returning the node
    /// sequence (inclusive of both endpoints) of one shortest path
    /// reachable within `max_hops` edges. `None` if no such path
    /// exists. `from == to` returns `Some(vec![from])` (a zero-hop
    /// path) regardless of `max_hops`. Endpoints not present in the
    /// graph yield `None`.
    ///
    /// Tie-breaking among equal-length paths follows insertion order
    /// of out-neighbors (BFS visits the first-discovered predecessor),
    /// keeping the output deterministic given the same edge insertion
    /// sequence.
    pub fn shortest_path(&self, from: &N, to: &N, max_hops: usize) -> Option<Vec<N>> {
        if !self.contains_node(from) || !self.contains_node(to) {
            return None;
        }
        if from == to {
            return Some(vec![from.clone()]);
        }

        let mut parent: BTreeMap<N, N> = BTreeMap::new();
        let mut visited: BTreeSet<N> = BTreeSet::new();
        let mut queue: VecDeque<(N, usize)> = VecDeque::new();
        visited.insert(from.clone());
        queue.push_back((from.clone(), 0));

        while let Some((node, hops)) = queue.pop_front() {
            if hops == max_hops {
                continue;
            }
            for next in self.neighbors(&node) {
                if !visited.insert(next.clone()) {
                    continue;
                }
                parent.insert(next.clone(), node.clone());
                if next == to {
                    return Some(reconstruct_path(&parent, from, to));
                }
                queue.push_back((next.clone(), hops + 1));
            }
        }
        None
    }

    /// Tarjan's algorithm for strongly connected components. Returns
    /// every SCC, including trivial size-1 components, with members
    /// sorted within each SCC and SCCs sorted by their first member —
    /// independent of DFS visitation order, so the output is stable
    /// across runs.
    ///
    /// Recursive DFS is intentional: component graphs are realistically
    /// tens-to-hundreds of nodes, well below stack limits, and the
    /// recursive form mirrors the canonical reference implementation
    /// for easy verification.
    pub fn strongly_connected_components(&self) -> Vec<Vec<N>> {
        let mut state = TarjanState::new();
        for node in self.nodes() {
            if !state.indices.contains_key(node) {
                state.strongconnect(self, node);
            }
        }
        state.into_sorted_components()
    }
}

fn reconstruct_path<N: Ord + Clone>(parent: &BTreeMap<N, N>, from: &N, to: &N) -> Vec<N> {
    let mut path = vec![to.clone()];
    let mut cursor = to.clone();
    while let Some(p) = parent.get(&cursor) {
        path.push(p.clone());
        if p == from {
            break;
        }
        cursor = p.clone();
    }
    path.reverse();
    path
}

struct TarjanState<N: Ord + Clone> {
    next_index: usize,
    indices: BTreeMap<N, usize>,
    lowlink: BTreeMap<N, usize>,
    on_stack: BTreeSet<N>,
    stack: Vec<N>,
    components: Vec<Vec<N>>,
}

impl<N: Ord + Clone> TarjanState<N> {
    fn new() -> Self {
        Self {
            next_index: 0,
            indices: BTreeMap::new(),
            lowlink: BTreeMap::new(),
            on_stack: BTreeSet::new(),
            stack: Vec::new(),
            components: Vec::new(),
        }
    }

    fn strongconnect(&mut self, graph: &DirectedGraph<N>, node: &N) {
        let idx = self.next_index;
        self.next_index += 1;
        self.indices.insert(node.clone(), idx);
        self.lowlink.insert(node.clone(), idx);
        self.stack.push(node.clone());
        self.on_stack.insert(node.clone());

        for next in graph.neighbors(node) {
            if !self.indices.contains_key(next) {
                self.strongconnect(graph, next);
                let next_low = self.lowlink[next];
                let cur_low = self.lowlink[node];
                if next_low < cur_low {
                    self.lowlink.insert(node.clone(), next_low);
                }
            } else if self.on_stack.contains(next) {
                let next_idx = self.indices[next];
                let cur_low = self.lowlink[node];
                if next_idx < cur_low {
                    self.lowlink.insert(node.clone(), next_idx);
                }
            }
        }

        if self.lowlink[node] == self.indices[node] {
            let mut component: Vec<N> = Vec::new();
            loop {
                let popped = self.stack.pop().expect("stack non-empty during SCC pop");
                self.on_stack.remove(&popped);
                let is_root = &popped == node;
                component.push(popped);
                if is_root {
                    break;
                }
            }
            self.components.push(component);
        }
    }

    fn into_sorted_components(mut self) -> Vec<Vec<N>> {
        for component in &mut self.components {
            component.sort();
        }
        self.components.sort();
        self.components
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

    // ---------- shortest_path tests ----------

    fn s(name: &str) -> String {
        name.to_string()
    }

    fn names(path: &[String]) -> Vec<&str> {
        path.iter().map(String::as_str).collect()
    }

    #[test]
    fn shortest_path_returns_chain_for_linear_graph() {
        // A → B → C; shortest path A to C is exactly [A, B, C].
        let g = graph_of_strs(&[("A", "B"), ("B", "C")]);
        let path = g.shortest_path(&s("A"), &s("C"), 10).unwrap();
        assert_eq!(names(&path), vec!["A", "B", "C"]);
    }

    #[test]
    fn shortest_path_picks_minimum_edge_count_in_diamond() {
        // A → B → D and A → C → D and A → D direct: 1-hop wins.
        let g = graph_of_strs(&[("A", "B"), ("B", "D"), ("A", "C"), ("C", "D"), ("A", "D")]);
        let path = g.shortest_path(&s("A"), &s("D"), 10).unwrap();
        assert_eq!(names(&path), vec!["A", "D"]);
    }

    #[test]
    fn shortest_path_for_zero_hop_self_target_is_singleton() {
        // from == to: a path of length 0 exists irrespective of max_hops.
        let g = graph_of_strs(&[("A", "B")]);
        let path = g.shortest_path(&s("A"), &s("A"), 0).unwrap();
        assert_eq!(names(&path), vec!["A"]);
    }

    #[test]
    fn shortest_path_returns_none_when_unreachable() {
        // A → B; C is in graph (isolated) but not reachable from A.
        let mut g = graph_of_strs(&[("A", "B")]);
        g.add_node(s("C"));
        assert!(g.shortest_path(&s("A"), &s("C"), 10).is_none());
    }

    #[test]
    fn shortest_path_returns_none_when_path_exceeds_hop_limit() {
        // A → B → C → D; with max_hops=2 the only path is 3 hops, so None.
        let g = graph_of_strs(&[("A", "B"), ("B", "C"), ("C", "D")]);
        assert!(g.shortest_path(&s("A"), &s("D"), 2).is_none());
        // max_hops=3 admits the path.
        let path = g.shortest_path(&s("A"), &s("D"), 3).unwrap();
        assert_eq!(names(&path), vec!["A", "B", "C", "D"]);
    }

    #[test]
    fn shortest_path_respects_directed_edges() {
        // A → B exists; B → A does not. Path B to A must fail.
        let g = graph_of_strs(&[("A", "B")]);
        assert!(g.shortest_path(&s("B"), &s("A"), 10).is_none());
    }

    #[test]
    fn shortest_path_terminates_on_cycles_without_revisiting() {
        // A → B → C → A → ... visiting A twice would loop forever; BFS
        // visited set prevents it. Path A to C is [A, B, C].
        let g = graph_of_strs(&[("A", "B"), ("B", "C"), ("C", "A")]);
        let path = g.shortest_path(&s("A"), &s("C"), 10).unwrap();
        assert_eq!(names(&path), vec!["A", "B", "C"]);
    }

    #[test]
    fn shortest_path_returns_none_for_unknown_endpoint() {
        let g = graph_of_strs(&[("A", "B")]);
        assert!(g.shortest_path(&s("A"), &s("ghost"), 10).is_none());
        assert!(g.shortest_path(&s("ghost"), &s("B"), 10).is_none());
    }

    #[test]
    fn shortest_path_with_max_hops_zero_only_satisfies_self() {
        let g = graph_of_strs(&[("A", "B")]);
        // from != to and 0 hops: impossible.
        assert!(g.shortest_path(&s("A"), &s("B"), 0).is_none());
        // from == to and 0 hops: trivially the singleton path.
        let path = g.shortest_path(&s("A"), &s("A"), 0).unwrap();
        assert_eq!(names(&path), vec!["A"]);
    }

    // ---------- strongly_connected_components tests ----------

    fn sccs_as_str(g: &DirectedGraph<String>) -> Vec<Vec<String>> {
        g.strongly_connected_components()
    }

    fn scc_view(sccs: &[Vec<String>]) -> Vec<Vec<&str>> {
        sccs.iter()
            .map(|c| c.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn scc_on_empty_graph_is_empty() {
        let g: DirectedGraph<String> = DirectedGraph::new();
        assert!(g.strongly_connected_components().is_empty());
    }

    #[test]
    fn scc_on_dag_yields_only_singletons() {
        // A → B → C, plus isolated D. No cycles, so every SCC is a
        // single node.
        let mut g = graph_of_strs(&[("A", "B"), ("B", "C")]);
        g.add_node(s("D"));
        let sccs = sccs_as_str(&g);
        assert_eq!(scc_view(&sccs), vec![vec!["A"], vec!["B"], vec!["C"], vec!["D"]]);
    }

    #[test]
    fn scc_on_simple_cycle_is_one_component() {
        // A → B → C → A: one SCC of size 3.
        let g = graph_of_strs(&[("A", "B"), ("B", "C"), ("C", "A")]);
        let sccs = sccs_as_str(&g);
        assert_eq!(scc_view(&sccs), vec![vec!["A", "B", "C"]]);
    }

    #[test]
    fn scc_separates_disjoint_components() {
        // Two cycles, no edges between: two non-trivial SCCs.
        let g = graph_of_strs(&[
            ("A", "B"),
            ("B", "A"),
            ("X", "Y"),
            ("Y", "Z"),
            ("Z", "X"),
        ]);
        let sccs = sccs_as_str(&g);
        assert_eq!(scc_view(&sccs), vec![vec!["A", "B"], vec!["X", "Y", "Z"]]);
    }

    #[test]
    fn scc_finds_cycle_inside_a_larger_dag() {
        // A → B → C → B (cycle B-C), C → D. SCCs: {A}, {B, C}, {D}.
        let g = graph_of_strs(&[("A", "B"), ("B", "C"), ("C", "B"), ("C", "D")]);
        let sccs = sccs_as_str(&g);
        assert_eq!(scc_view(&sccs), vec![vec!["A"], vec!["B", "C"], vec!["D"]]);
    }

    #[test]
    fn scc_partitions_every_node_exactly_once() {
        // Sanity: union of all SCCs == node set, and SCCs are disjoint.
        let g = graph_of_strs(&[("A", "B"), ("B", "A"), ("B", "C"), ("C", "D"), ("D", "C")]);
        let sccs = g.strongly_connected_components();
        let mut union: BTreeSet<&String> = BTreeSet::new();
        let mut total = 0;
        for c in &sccs {
            for n in c {
                assert!(union.insert(n), "node {n} appeared in two SCCs");
                total += 1;
            }
        }
        assert_eq!(total, g.node_count());
        let nodes: BTreeSet<&String> = g.nodes().collect();
        assert_eq!(union, nodes);
    }

    #[test]
    fn scc_treats_isolated_nodes_as_singleton_components() {
        let mut g: DirectedGraph<String> = DirectedGraph::new();
        g.add_node(s("solo"));
        assert_eq!(scc_view(&sccs_as_str(&g)), vec![vec!["solo"]]);
    }

    #[test]
    fn scc_handles_self_loop_as_singleton_component() {
        // A self-loop A → A is a 1-node SCC. Tarjan still emits it as
        // a singleton; the loop doesn't promote it to non-trivial.
        let g = graph_of_strs(&[("A", "A")]);
        assert_eq!(scc_view(&sccs_as_str(&g)), vec![vec!["A"]]);
    }

    #[test]
    fn scc_output_is_deterministic_regardless_of_insertion_order() {
        // Same logical graph, different edge insertion order. SCCs
        // must come out in the same sorted form.
        let g1 = graph_of_strs(&[("X", "Y"), ("Y", "X"), ("A", "B"), ("B", "A")]);
        let g2 = graph_of_strs(&[("A", "B"), ("B", "A"), ("X", "Y"), ("Y", "X")]);
        let sccs1 = sccs_as_str(&g1);
        let sccs2 = sccs_as_str(&g2);
        assert_eq!(scc_view(&sccs1), scc_view(&sccs2));
        assert_eq!(scc_view(&sccs1), vec![vec!["A", "B"], vec!["X", "Y"]]);
    }
}
