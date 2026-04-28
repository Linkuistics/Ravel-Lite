use std::collections::HashMap;

use petgraph::algo::{dijkstra, tarjan_scc, toposort};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::{Bfs, EdgeRef};
use petgraph::Direction;

use crate::item::{Item, ItemId};
use crate::store::Store;

/// Default edge function: walk every justification that references another item id.
/// Edges are drawn from the justifying item to the referenced item.
pub fn default_edges(item: &Item) -> Vec<ItemId> {
    item.justifications
        .iter()
        .filter_map(|j| j.references_item().cloned())
        .collect()
}

pub struct GraphView<'s> {
    store: &'s Store,
    graph: DiGraph<ItemId, ()>,
    node_for: HashMap<ItemId, NodeIndex>,
}

impl<'s> GraphView<'s> {
    /// Build a graph over the store using `default_edges` (justification refs).
    pub fn new(store: &'s Store) -> Self {
        Self::with_edges(store, default_edges)
    }

    /// Build a graph over the store using a caller-supplied edge function.
    pub fn with_edges<F>(store: &'s Store, edge_fn: F) -> Self
    where
        F: Fn(&Item) -> Vec<ItemId>,
    {
        let mut graph = DiGraph::<ItemId, ()>::new();
        let mut node_for = HashMap::new();
        for item in store.iter() {
            let n = graph.add_node(item.id.clone());
            node_for.insert(item.id.clone(), n);
        }
        for item in store.iter() {
            let from = node_for[&item.id];
            for to_id in edge_fn(item) {
                if let Some(&to) = node_for.get(&to_id) {
                    graph.add_edge(from, to, ());
                }
            }
        }
        Self {
            store,
            graph,
            node_for,
        }
    }

    pub fn store(&self) -> &Store {
        self.store
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    fn node(&self, id: &str) -> Option<NodeIndex> {
        self.node_for.get(id).copied()
    }

    fn id_of(&self, n: NodeIndex) -> &ItemId {
        &self.graph[n]
    }
}

/// Shortest directed path from `from` to `to` (unweighted).
pub fn shortest_path(view: &GraphView, from: &str, to: &str) -> Option<Vec<ItemId>> {
    let src = view.node(from)?;
    let dst = view.node(to)?;
    let scores = dijkstra(&view.graph, src, Some(dst), |_| 1usize);
    if !scores.contains_key(&dst) {
        return None;
    }
    // Reconstruct via BFS predecessor map.
    let mut pred: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut seen = vec![false; view.graph.node_count()];
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(src);
    seen[src.index()] = true;
    while let Some(n) = queue.pop_front() {
        if n == dst {
            break;
        }
        for e in view.graph.edges_directed(n, Direction::Outgoing) {
            let m = e.target();
            if !seen[m.index()] {
                seen[m.index()] = true;
                pred.insert(m, n);
                queue.push_back(m);
            }
        }
    }
    let mut path = vec![view.id_of(dst).clone()];
    let mut cur = dst;
    while cur != src {
        let p = *pred.get(&cur)?;
        path.push(view.id_of(p).clone());
        cur = p;
    }
    path.reverse();
    Some(path)
}

/// BFS reachable set from `start` up to `max_depth` (inclusive).
/// Depth 0 returns just the start node; `None` for unbounded BFS.
pub fn bfs_subgraph(view: &GraphView, start: &str, max_depth: Option<usize>) -> Vec<ItemId> {
    let Some(src) = view.node(start) else {
        return vec![];
    };
    if max_depth == Some(0) {
        return vec![view.id_of(src).clone()];
    }
    let mut bfs = Bfs::new(&view.graph, src);
    let mut depth: HashMap<NodeIndex, usize> = HashMap::new();
    depth.insert(src, 0);
    let mut out = Vec::new();
    while let Some(n) = bfs.next(&view.graph) {
        let d = *depth.get(&n).unwrap_or(&0);
        if let Some(limit) = max_depth {
            if d > limit {
                continue;
            }
        }
        out.push(view.id_of(n).clone());
        for e in view.graph.edges_directed(n, Direction::Outgoing) {
            depth.entry(e.target()).or_insert(d + 1);
        }
    }
    out
}

/// Strongly-connected components, each as a list of item ids.
pub fn strongly_connected_components(view: &GraphView) -> Vec<Vec<ItemId>> {
    tarjan_scc(&view.graph)
        .into_iter()
        .map(|scc| scc.into_iter().map(|n| view.id_of(n).clone()).collect())
        .collect()
}

/// Topological sort. Returns `Err(id)` on the first node detected to be in a cycle.
pub fn topological_sort(view: &GraphView) -> Result<Vec<ItemId>, ItemId> {
    match toposort(&view.graph, None) {
        Ok(order) => Ok(order.into_iter().map(|n| view.id_of(n).clone()).collect()),
        Err(cycle) => Err(view.id_of(cycle.node_id()).clone()),
    }
}

/// Articulation points of the graph viewed as undirected. Hand-rolled because
/// petgraph 0.6 does not export this in its stable surface; the algorithm is
/// the standard low-link DFS.
pub fn articulation_points(view: &GraphView) -> Vec<ItemId> {
    let n = view.graph.node_count();
    if n == 0 {
        return vec![];
    }
    // Build undirected adjacency.
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in view.graph.edge_references() {
        let a = e.source().index();
        let b = e.target().index();
        if !adj[a].contains(&b) {
            adj[a].push(b);
        }
        if !adj[b].contains(&a) {
            adj[b].push(a);
        }
    }

    let mut visited = vec![false; n];
    let mut disc = vec![0usize; n];
    let mut low = vec![0usize; n];
    let mut parent = vec![usize::MAX; n];
    let mut is_ap = vec![false; n];
    let mut timer = 0usize;

    fn dfs(
        u: usize,
        adj: &[Vec<usize>],
        visited: &mut [bool],
        disc: &mut [usize],
        low: &mut [usize],
        parent: &mut [usize],
        is_ap: &mut [bool],
        timer: &mut usize,
    ) {
        visited[u] = true;
        *timer += 1;
        disc[u] = *timer;
        low[u] = *timer;
        let mut children = 0usize;
        for &v in &adj[u] {
            if !visited[v] {
                children += 1;
                parent[v] = u;
                dfs(v, adj, visited, disc, low, parent, is_ap, timer);
                low[u] = low[u].min(low[v]);
                if parent[u] == usize::MAX && children > 1 {
                    is_ap[u] = true;
                }
                if parent[u] != usize::MAX && low[v] >= disc[u] {
                    is_ap[u] = true;
                }
            } else if v != parent[u] {
                low[u] = low[u].min(disc[v]);
            }
        }
    }

    for u in 0..n {
        if !visited[u] {
            dfs(
                u,
                &adj,
                &mut visited,
                &mut disc,
                &mut low,
                &mut parent,
                &mut is_ap,
                &mut timer,
            );
        }
    }

    (0..n)
        .filter(|&i| is_ap[i])
        .map(|i| view.graph[NodeIndex::new(i)].clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Item;
    use crate::justification::Justification;

    fn item(id: &str, refs: Vec<&str>) -> Item {
        Item {
            id: id.into(),
            kind: "test".into(),
            claim: "c".into(),
            justifications: refs
                .into_iter()
                .map(|r| Justification::ServesIntent {
                    intent_id: r.into(),
                })
                .collect(),
            status: "active".into(),
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: "t".into(),
            authored_in: "test".into(),
        }
    }

    fn store_with(items: Vec<Item>) -> Store {
        let mut s = Store::new();
        for i in items {
            s.insert(i).unwrap();
        }
        s
    }

    #[test]
    fn shortest_path_finds_three_hop_chain() {
        // a -> b -> c -> d
        let store = store_with(vec![
            item("a", vec!["b"]),
            item("b", vec!["c"]),
            item("c", vec!["d"]),
            item("d", vec![]),
        ]);
        let view = GraphView::new(&store);
        let path = shortest_path(&view, "a", "d").unwrap();
        assert_eq!(path, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn shortest_path_returns_none_when_unreachable() {
        let store = store_with(vec![item("a", vec![]), item("b", vec![])]);
        let view = GraphView::new(&store);
        assert!(shortest_path(&view, "a", "b").is_none());
    }

    #[test]
    fn bfs_subgraph_respects_max_depth() {
        // a -> b -> c
        let store = store_with(vec![
            item("a", vec!["b"]),
            item("b", vec!["c"]),
            item("c", vec![]),
        ]);
        let view = GraphView::new(&store);
        let nodes = bfs_subgraph(&view, "a", Some(1));
        assert!(nodes.contains(&"a".to_string()));
        assert!(nodes.contains(&"b".to_string()));
        assert!(!nodes.contains(&"c".to_string()));
    }

    #[test]
    fn scc_finds_cycle() {
        // a -> b -> a forms one SCC; c is its own SCC.
        let store = store_with(vec![
            item("a", vec!["b"]),
            item("b", vec!["a"]),
            item("c", vec![]),
        ]);
        let view = GraphView::new(&store);
        let sccs = strongly_connected_components(&view);
        let big = sccs.iter().find(|s| s.len() == 2).unwrap();
        assert!(big.contains(&"a".to_string()) && big.contains(&"b".to_string()));
    }

    #[test]
    fn topo_sort_orders_a_dag() {
        // a -> b, a -> c, b -> d, c -> d
        let store = store_with(vec![
            item("a", vec!["b", "c"]),
            item("b", vec!["d"]),
            item("c", vec!["d"]),
            item("d", vec![]),
        ]);
        let view = GraphView::new(&store);
        let order = topological_sort(&view).unwrap();
        let pos = |id: &str| order.iter().position(|x| x == id).unwrap();
        assert!(pos("a") < pos("b"));
        assert!(pos("a") < pos("c"));
        assert!(pos("b") < pos("d"));
        assert!(pos("c") < pos("d"));
    }

    #[test]
    fn topo_sort_errors_on_cycle() {
        let store = store_with(vec![item("a", vec!["b"]), item("b", vec!["a"])]);
        let view = GraphView::new(&store);
        assert!(topological_sort(&view).is_err());
    }

    #[test]
    fn articulation_points_finds_central_node() {
        // a -- b -- c (linear undirected). b is the articulation point.
        let store = store_with(vec![
            item("a", vec!["b"]),
            item("b", vec!["c"]),
            item("c", vec![]),
        ]);
        let view = GraphView::new(&store);
        let aps = articulation_points(&view);
        assert_eq!(aps, vec!["b"]);
    }
}
