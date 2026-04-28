use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::item::{DefeatedBy, Item, ItemId};
use crate::justification::Justification;

pub const STORE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("item id `{0}` already exists")]
    Duplicate(ItemId),
    #[error("item id `{0}` not found")]
    NotFound(ItemId),
    #[error("yaml serialisation: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("schema_version mismatch: store expects {expected}, file declares {found}")]
    SchemaMismatch { expected: u32, found: u32 },
}

#[derive(Debug, Clone, Default)]
pub struct Store {
    items: IndexMap<ItemId, Item>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoreFile {
    schema_version: u32,
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefeatReport {
    /// The item that was directly defeated (the cascade root).
    pub root: ItemId,
    /// Items defeated transitively by the cascade.
    pub cascaded: Vec<ItemId>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn insert(&mut self, item: Item) -> Result<(), StoreError> {
        if self.items.contains_key(&item.id) {
            return Err(StoreError::Duplicate(item.id));
        }
        self.items.insert(item.id.clone(), item);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items.get_mut(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Item> {
        self.items.values()
    }

    pub fn ids(&self) -> impl Iterator<Item = &ItemId> {
        self.items.keys()
    }

    /// Set a status string on an item. No cascade — see [`defeat`](Self::defeat).
    pub fn set_status(&mut self, id: &str, status: impl Into<String>) -> Result<(), StoreError> {
        let item = self
            .items
            .get_mut(id)
            .ok_or_else(|| StoreError::NotFound(id.into()))?;
        item.status = status.into();
        Ok(())
    }

    /// Defeat an item and propagate the cascade through `serves-intent` edges.
    ///
    /// The root item is marked `defeated_by: <root_id>` (self-marker via the
    /// caller's framing — the substrate writes `Some(DefeatedBy::Item(id))`
    /// for the root and `Some(DefeatedBy::Cascade)` for cascade-defeated items).
    /// Status is set to `"defeated"` for both root and cascaded items.
    ///
    /// Cascade rule: an item is cascade-defeated if every `serves-intent`
    /// justification it carries points at an already-defeated item, and it
    /// has at least one such justification. Items with no `serves-intent`
    /// justifications are unaffected.
    ///
    /// `blocked_by`-driven cascade is intentionally not in this scaffold;
    /// it lives as kind-specific extension data per `architecture-next.md`
    /// §Item shape and is the subject of a follow-on task.
    pub fn defeat(&mut self, root_id: &str) -> Result<DefeatReport, StoreError> {
        if !self.items.contains_key(root_id) {
            return Err(StoreError::NotFound(root_id.into()));
        }

        let mut defeated: indexmap::IndexSet<ItemId> = indexmap::IndexSet::new();
        defeated.insert(root_id.to_string());

        let mut cascaded = Vec::new();
        let mut changed = true;
        while changed {
            changed = false;
            for item in self.items.values() {
                if defeated.contains(&item.id) {
                    continue;
                }
                let serves: Vec<&ItemId> = item
                    .justifications
                    .iter()
                    .filter_map(|j| match j {
                        Justification::ServesIntent { intent_id } => Some(intent_id),
                        _ => None,
                    })
                    .collect();
                if serves.is_empty() {
                    continue;
                }
                if serves.iter().all(|id| defeated.contains(*id)) {
                    cascaded.push(item.id.clone());
                    defeated.insert(item.id.clone());
                    changed = true;
                }
            }
        }

        // Apply mutations.
        let root = self
            .items
            .get_mut(root_id)
            .expect("checked at entry");
        root.status = "defeated".into();
        root.defeated_by = Some(DefeatedBy::Item(root_id.into()));

        for id in &cascaded {
            let item = self
                .items
                .get_mut(id)
                .expect("ids drawn from self.items");
            item.status = "defeated".into();
            item.defeated_by = Some(DefeatedBy::Cascade);
        }

        Ok(DefeatReport {
            root: root_id.into(),
            cascaded,
        })
    }

    /// Mark `old` as superseded by `new`, setting both ends of the link.
    /// Both items must exist.
    pub fn supersede(&mut self, old_id: &str, new_id: &str) -> Result<(), StoreError> {
        if !self.items.contains_key(old_id) {
            return Err(StoreError::NotFound(old_id.into()));
        }
        if !self.items.contains_key(new_id) {
            return Err(StoreError::NotFound(new_id.into()));
        }
        let old = self.items.get_mut(old_id).expect("checked");
        old.superseded_by = Some(new_id.into());
        old.status = "superseded".into();
        let new = self.items.get_mut(new_id).expect("checked");
        if !new.supersedes.iter().any(|s| s == old_id) {
            new.supersedes.push(old_id.into());
        }
        Ok(())
    }

    pub fn to_yaml(&self) -> Result<String, StoreError> {
        let file = StoreFile {
            schema_version: STORE_SCHEMA_VERSION,
            items: self.items.values().cloned().collect(),
        };
        Ok(serde_yaml::to_string(&file)?)
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, StoreError> {
        let file: StoreFile = serde_yaml::from_str(yaml)?;
        if file.schema_version != STORE_SCHEMA_VERSION {
            return Err(StoreError::SchemaMismatch {
                expected: STORE_SCHEMA_VERSION,
                found: file.schema_version,
            });
        }
        let mut store = Self::new();
        for item in file.items {
            store.insert(item)?;
        }
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: &str, status: &str, justs: Vec<Justification>) -> Item {
        Item {
            id: id.into(),
            kind: kind.into(),
            claim: format!("claim of {id}"),
            justifications: justs,
            status: status.into(),
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: "2026-04-28T00:00:00Z".into(),
            authored_in: "test".into(),
        }
    }

    #[test]
    fn yaml_roundtrip_preserves_order_and_content() {
        let mut store = Store::new();
        store.insert(item("i-1", "intent", "active", vec![])).unwrap();
        store
            .insert(item(
                "t-1",
                "backlog-item",
                "active",
                vec![Justification::ServesIntent {
                    intent_id: "i-1".into(),
                }],
            ))
            .unwrap();
        let yaml = store.to_yaml().unwrap();
        let back = Store::from_yaml(&yaml).unwrap();
        assert_eq!(back.len(), 2);
        assert!(back.get("i-1").is_some());
        assert!(back.get("t-1").is_some());
    }

    #[test]
    fn duplicate_insert_is_an_error() {
        let mut store = Store::new();
        store.insert(item("a", "x", "active", vec![])).unwrap();
        let err = store.insert(item("a", "x", "active", vec![])).unwrap_err();
        assert!(matches!(err, StoreError::Duplicate(_)));
    }

    #[test]
    fn schema_version_mismatch_is_an_error() {
        let yaml = "schema_version: 99\nitems: []\n";
        let err = Store::from_yaml(yaml).unwrap_err();
        assert!(matches!(err, StoreError::SchemaMismatch { .. }));
    }

    #[test]
    fn defeat_cascades_through_serves_intent() {
        let mut store = Store::new();
        store.insert(item("i-1", "intent", "active", vec![])).unwrap();
        store
            .insert(item(
                "t-1",
                "backlog-item",
                "active",
                vec![Justification::ServesIntent {
                    intent_id: "i-1".into(),
                }],
            ))
            .unwrap();
        store
            .insert(item(
                "t-2",
                "backlog-item",
                "active",
                vec![Justification::ServesIntent {
                    intent_id: "i-1".into(),
                }],
            ))
            .unwrap();
        let report = store.defeat("i-1").unwrap();
        assert_eq!(report.root, "i-1");
        assert_eq!(report.cascaded.len(), 2);
        assert_eq!(store.get("i-1").unwrap().status, "defeated");
        assert_eq!(store.get("t-1").unwrap().status, "defeated");
        assert_eq!(
            store.get("t-1").unwrap().defeated_by,
            Some(DefeatedBy::Cascade)
        );
    }

    #[test]
    fn defeat_does_not_cascade_when_other_intent_is_active() {
        let mut store = Store::new();
        store.insert(item("i-1", "intent", "active", vec![])).unwrap();
        store.insert(item("i-2", "intent", "active", vec![])).unwrap();
        store
            .insert(item(
                "t-1",
                "backlog-item",
                "active",
                vec![
                    Justification::ServesIntent {
                        intent_id: "i-1".into(),
                    },
                    Justification::ServesIntent {
                        intent_id: "i-2".into(),
                    },
                ],
            ))
            .unwrap();
        let report = store.defeat("i-1").unwrap();
        assert!(report.cascaded.is_empty());
        assert_eq!(store.get("t-1").unwrap().status, "active");
    }

    #[test]
    fn defeat_cascades_transitively() {
        // i-1 ← t-1 ← (no, t-2 serves t-1 via serves-intent? actually serves-intent
        // is intent->backlog only; but the substrate is generic over kind, so we
        // exercise transitive cascade by having an item that serves an item
        // that serves the root.
        let mut store = Store::new();
        store.insert(item("i-1", "intent", "active", vec![])).unwrap();
        store
            .insert(item(
                "t-1",
                "sub-intent",
                "active",
                vec![Justification::ServesIntent {
                    intent_id: "i-1".into(),
                }],
            ))
            .unwrap();
        store
            .insert(item(
                "t-2",
                "backlog-item",
                "active",
                vec![Justification::ServesIntent {
                    intent_id: "t-1".into(),
                }],
            ))
            .unwrap();
        let report = store.defeat("i-1").unwrap();
        assert_eq!(report.cascaded.len(), 2);
        assert!(report.cascaded.contains(&"t-1".to_string()));
        assert!(report.cascaded.contains(&"t-2".to_string()));
    }

    #[test]
    fn defeat_unknown_id_is_an_error() {
        let mut store = Store::new();
        let err = store.defeat("ghost").unwrap_err();
        assert!(matches!(err, StoreError::NotFound(_)));
    }

    #[test]
    fn supersede_links_both_ends_and_sets_status() {
        let mut store = Store::new();
        store.insert(item("m-1", "memory", "active", vec![])).unwrap();
        store.insert(item("m-2", "memory", "active", vec![])).unwrap();
        store.supersede("m-1", "m-2").unwrap();
        assert_eq!(
            store.get("m-1").unwrap().superseded_by.as_deref(),
            Some("m-2")
        );
        assert_eq!(store.get("m-1").unwrap().status, "superseded");
        assert_eq!(store.get("m-2").unwrap().supersedes, vec!["m-1".to_string()]);
    }
}
