//! Typed knowledge-graph + truth-maintenance substrate.
//!
//! Plan state (intents, backlog items, memory entries, findings) and the
//! catalog graph share this substrate. Items have ids, kinds, claims,
//! structured justifications, and statuses; edges are typed; status mutations
//! cascade deterministically along justification edges.
//!
//! Domain consumers register their own kinds, statuses, and rule programs;
//! this crate is generic and knows nothing about ravel-lite or the catalog
//! vocabulary.
//!
//! See `docs/architecture-next.md` §Knowledge substrate (TMS + KG) for the
//! design rationale.

pub mod algorithms;
pub mod datalog;
pub mod item;
pub mod justification;
pub mod store;

pub use algorithms::{
    articulation_points, bfs_subgraph, default_edges, shortest_path,
    strongly_connected_components, topological_sort, GraphView,
};
pub use datalog::SmokeProgram;
pub use item::{DefeatedBy, Item, ItemId};
pub use justification::Justification;
pub use store::{DefeatReport, Store, StoreError, STORE_SCHEMA_VERSION};
