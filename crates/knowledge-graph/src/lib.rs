//! Typed knowledge-graph + truth-maintenance substrate.
//!
//! Plan state (intents, backlog items, memory entries, findings) and the
//! catalog graph share this substrate. Items have ids, typed kinds and
//! statuses, structured justifications; edges are typed; status mutations
//! cascade deterministically along justification edges.
//!
//! Domain consumers register their vocabulary by implementing
//! [`ItemKind`] and [`ItemStatus`] for tag and status types of their
//! own. The substrate stays vocabulary-agnostic — it knows nothing
//! about ravel-lite or the catalog.
//!
//! See `docs/architecture-next.md` §Knowledge substrate (TMS + KG) for
//! the design rationale.

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
pub use item::{DefeatedBy, Item, ItemId, ItemKind, ItemStatus, KindMarker};
pub use justification::Justification;
pub use store::{cascade_serves_intent, DefeatReport, Store, StoreError, STORE_SCHEMA_VERSION};
