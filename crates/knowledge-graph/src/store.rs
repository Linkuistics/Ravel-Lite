use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::item::{DefeatedBy, Item, ItemId, ItemKind, ItemStatus};
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
    #[error(
        "illegal status mutation for `{id}` ({kind}): `{from}` → `{to}` is not in the transition table"
    )]
    IllegalTransition {
        id: ItemId,
        kind: &'static str,
        from: &'static str,
        to: &'static str,
    },
}

/// Per-kind item store. One `Store<K>` holds items of exactly one
/// kind. Cross-kind cascade (e.g., intent → backlog item via
/// `serves-intent`) is expressed as a separate function over two
/// stores; see [`cascade_serves_intent`].
///
/// `Debug`, `Clone`, `Default` are written manually for the same
/// reason as on `Item<K>`: the auto-derived bounds would require
/// `K: Debug + Clone + Default`, but `K` only appears as a phantom
/// tag and the field types are unconditionally `Debug + Clone`.
pub struct Store<K: ItemKind> {
    items: IndexMap<ItemId, Item<K>>,
}

impl<K: ItemKind> Default for Store<K> {
    fn default() -> Self {
        Self {
            items: IndexMap::new(),
        }
    }
}

impl<K: ItemKind> Clone for Store<K> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
        }
    }
}

impl<K: ItemKind> std::fmt::Debug for Store<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("kind", &K::KIND_STR)
            .field("items", &self.items)
            .finish()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(
    serialize = "K::Status: Serialize",
    deserialize = "K::Status: serde::de::DeserializeOwned"
))]
struct StoreFile<K: ItemKind> {
    schema_version: u32,
    #[serde(default = "Vec::new")]
    items: Vec<Item<K>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DefeatReport {
    /// The item that was directly defeated (the cascade root).
    pub root: ItemId,
    /// Items defeated transitively by the cascade.
    pub cascaded: Vec<ItemId>,
}

impl<K: ItemKind> Store<K> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn insert(&mut self, item: Item<K>) -> Result<(), StoreError> {
        if self.items.contains_key(&item.id) {
            return Err(StoreError::Duplicate(item.id));
        }
        self.items.insert(item.id.clone(), item);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&Item<K>> {
        self.items.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Item<K>> {
        self.items.get_mut(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Item<K>> {
        self.items.values()
    }

    pub fn ids(&self) -> impl Iterator<Item = &ItemId> {
        self.items.keys()
    }

    /// Set a status with transition validation. Self-transitions are
    /// accepted as no-ops. Use [`defeat`](Self::defeat) for status
    /// changes that should propagate through the cascade.
    pub fn set_status(&mut self, id: &str, new_status: K::Status) -> Result<(), StoreError> {
        let item = self
            .items
            .get_mut(id)
            .ok_or_else(|| StoreError::NotFound(id.into()))?;
        let from = item.status;
        if from == new_status {
            return Ok(());
        }
        if !K::Status::transitions()
            .iter()
            .any(|(a, b)| *a == from && *b == new_status)
        {
            return Err(StoreError::IllegalTransition {
                id: id.into(),
                kind: K::KIND_STR,
                from: from.as_str(),
                to: new_status.as_str(),
            });
        }
        item.status = new_status;
        Ok(())
    }

    /// Defeat an item and propagate the cascade through `serves-intent`
    /// edges that point to other items **in this same store** (e.g., a
    /// sub-intent that serves a parent intent of the same kind). Cross-
    /// kind cascade — the much more common case for plan KGs — is
    /// expressed as a separate function over two stores; see
    /// [`cascade_serves_intent`].
    ///
    /// `defeated_status` is the status enum value the consumer uses to
    /// represent "defeated"; the substrate is vocabulary-agnostic and
    /// cannot pick this value itself.
    pub fn defeat(
        &mut self,
        root_id: &str,
        defeated_status: K::Status,
    ) -> Result<DefeatReport, StoreError> {
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

        let root = self.items.get_mut(root_id).expect("checked at entry");
        root.status = defeated_status;
        root.defeated_by = Some(DefeatedBy::Item(root_id.into()));

        for id in &cascaded {
            let item = self.items.get_mut(id).expect("ids drawn from self.items");
            item.status = defeated_status;
            item.defeated_by = Some(DefeatedBy::Cascade);
        }

        Ok(DefeatReport {
            root: root_id.into(),
            cascaded,
        })
    }

    /// Mark `old` as superseded by `new`, setting both ends of the link.
    /// Both items must exist; `superseded_status` is the consumer's
    /// status enum value for "superseded".
    pub fn supersede(
        &mut self,
        old_id: &str,
        new_id: &str,
        superseded_status: K::Status,
    ) -> Result<(), StoreError> {
        if !self.items.contains_key(old_id) {
            return Err(StoreError::NotFound(old_id.into()));
        }
        if !self.items.contains_key(new_id) {
            return Err(StoreError::NotFound(new_id.into()));
        }
        let old = self.items.get_mut(old_id).expect("checked");
        old.superseded_by = Some(new_id.into());
        old.status = superseded_status;
        let new = self.items.get_mut(new_id).expect("checked");
        if !new.supersedes.iter().any(|s| s == old_id) {
            new.supersedes.push(old_id.into());
        }
        Ok(())
    }

    pub fn to_yaml(&self) -> Result<String, StoreError> {
        let file = StoreFile::<K> {
            schema_version: STORE_SCHEMA_VERSION,
            items: self.items.values().cloned().collect(),
        };
        Ok(serde_yaml::to_string(&file)?)
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, StoreError> {
        let file: StoreFile<K> = serde_yaml::from_str(yaml)?;
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

/// Cross-kind cascade along `serves-intent`.
///
/// For each item in `dependents` whose every `serves-intent`
/// justification points at an item in `parents` whose status is
/// terminal (per `ItemStatus::is_terminal`), defeat the dependent —
/// setting its status to `cascade_status` and `defeated_by:
/// cascade`. Items whose `serves-intent` ids do not all resolve to
/// `parents` (e.g., they reference a different store, or an unknown
/// id) are left untouched. Items with no `serves-intent`
/// justifications at all are likewise untouched.
///
/// Returns the ids of items that were cascade-defeated by this call.
pub fn cascade_serves_intent<P: ItemKind, C: ItemKind>(
    parents: &Store<P>,
    dependents: &mut Store<C>,
    cascade_status: C::Status,
) -> Vec<ItemId> {
    let mut newly_defeated: Vec<ItemId> = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        let candidates: Vec<ItemId> = dependents
            .iter()
            .filter(|item| !item.status.is_terminal())
            .filter_map(|item| {
                let serves: Vec<&ItemId> = item
                    .justifications
                    .iter()
                    .filter_map(|j| match j {
                        Justification::ServesIntent { intent_id } => Some(intent_id),
                        _ => None,
                    })
                    .collect();
                if serves.is_empty() {
                    return None;
                }
                let all_defeated_in_parents = serves.iter().all(|sid| {
                    parents
                        .get(sid)
                        .map(|p| p.status.is_terminal())
                        .unwrap_or(false)
                });
                if all_defeated_in_parents {
                    Some(item.id.clone())
                } else {
                    None
                }
            })
            .collect();
        for id in candidates {
            let item = dependents
                .get_mut(&id)
                .expect("id drawn from dependents iter");
            item.status = cascade_status;
            item.defeated_by = Some(DefeatedBy::Cascade);
            newly_defeated.push(id);
            changed = true;
        }
    }
    newly_defeated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::test_support::*;

    #[test]
    fn yaml_roundtrip_preserves_order_and_content() {
        let mut store = Store::<TestKind>::new();
        store.insert(item("a", TestStatus::Active, vec![])).unwrap();
        store
            .insert(item(
                "b",
                TestStatus::Active,
                vec![Justification::ServesIntent {
                    intent_id: "a".into(),
                }],
            ))
            .unwrap();
        let yaml = store.to_yaml().unwrap();
        let back = Store::<TestKind>::from_yaml(&yaml).unwrap();
        assert_eq!(back.len(), 2);
        assert!(back.get("a").is_some());
        assert!(back.get("b").is_some());
    }

    #[test]
    fn duplicate_insert_is_an_error() {
        let mut store = Store::<TestKind>::new();
        store.insert(item("a", TestStatus::Active, vec![])).unwrap();
        let err = store
            .insert(item("a", TestStatus::Active, vec![]))
            .unwrap_err();
        assert!(matches!(err, StoreError::Duplicate(_)));
    }

    #[test]
    fn schema_version_mismatch_is_an_error() {
        let yaml = "schema_version: 99\nitems: []\n";
        let err = Store::<TestKind>::from_yaml(yaml).unwrap_err();
        assert!(matches!(err, StoreError::SchemaMismatch { .. }));
    }

    #[test]
    fn intra_store_defeat_cascades_through_serves_intent() {
        // Sub-intent chain within one store: t-1 serves a, t-2 serves a.
        // Defeating `a` cascades to t-1 and t-2.
        let mut store = Store::<TestKind>::new();
        store.insert(item("a", TestStatus::Active, vec![])).unwrap();
        store
            .insert(item(
                "t-1",
                TestStatus::Active,
                vec![Justification::ServesIntent {
                    intent_id: "a".into(),
                }],
            ))
            .unwrap();
        store
            .insert(item(
                "t-2",
                TestStatus::Active,
                vec![Justification::ServesIntent {
                    intent_id: "a".into(),
                }],
            ))
            .unwrap();
        let report = store.defeat("a", TestStatus::Defeated).unwrap();
        assert_eq!(report.root, "a");
        assert_eq!(report.cascaded.len(), 2);
        assert_eq!(store.get("a").unwrap().status, TestStatus::Defeated);
        assert_eq!(store.get("t-1").unwrap().status, TestStatus::Defeated);
        assert_eq!(
            store.get("t-1").unwrap().defeated_by,
            Some(DefeatedBy::Cascade)
        );
    }

    #[test]
    fn intra_store_defeat_holds_when_other_intent_active() {
        let mut store = Store::<TestKind>::new();
        store.insert(item("a", TestStatus::Active, vec![])).unwrap();
        store.insert(item("b", TestStatus::Active, vec![])).unwrap();
        store
            .insert(item(
                "t-1",
                TestStatus::Active,
                vec![
                    Justification::ServesIntent {
                        intent_id: "a".into(),
                    },
                    Justification::ServesIntent {
                        intent_id: "b".into(),
                    },
                ],
            ))
            .unwrap();
        let report = store.defeat("a", TestStatus::Defeated).unwrap();
        assert!(report.cascaded.is_empty());
        assert_eq!(store.get("t-1").unwrap().status, TestStatus::Active);
    }

    #[test]
    fn intra_store_defeat_cascades_transitively() {
        let mut store = Store::<TestKind>::new();
        store.insert(item("a", TestStatus::Active, vec![])).unwrap();
        store
            .insert(item(
                "b",
                TestStatus::Active,
                vec![Justification::ServesIntent {
                    intent_id: "a".into(),
                }],
            ))
            .unwrap();
        store
            .insert(item(
                "c",
                TestStatus::Active,
                vec![Justification::ServesIntent {
                    intent_id: "b".into(),
                }],
            ))
            .unwrap();
        let report = store.defeat("a", TestStatus::Defeated).unwrap();
        assert_eq!(report.cascaded.len(), 2);
        assert!(report.cascaded.contains(&"b".to_string()));
        assert!(report.cascaded.contains(&"c".to_string()));
    }

    #[test]
    fn defeat_unknown_id_is_an_error() {
        let mut store = Store::<TestKind>::new();
        let err = store.defeat("ghost", TestStatus::Defeated).unwrap_err();
        assert!(matches!(err, StoreError::NotFound(_)));
    }

    #[test]
    fn supersede_links_both_ends_and_sets_status() {
        let mut store = Store::<TestKind>::new();
        store.insert(item("m-1", TestStatus::Active, vec![])).unwrap();
        store.insert(item("m-2", TestStatus::Active, vec![])).unwrap();
        store
            .supersede("m-1", "m-2", TestStatus::Superseded)
            .unwrap();
        assert_eq!(
            store.get("m-1").unwrap().superseded_by.as_deref(),
            Some("m-2")
        );
        assert_eq!(store.get("m-1").unwrap().status, TestStatus::Superseded);
        assert_eq!(store.get("m-2").unwrap().supersedes, vec!["m-1".to_string()]);
    }

    #[test]
    fn set_status_rejects_illegal_transition() {
        let mut store = Store::<TestKind>::new();
        store.insert(item("t-1", TestStatus::Done, vec![])).unwrap();
        // Done is terminal — can't move to Active.
        let err = store.set_status("t-1", TestStatus::Active).unwrap_err();
        assert!(matches!(err, StoreError::IllegalTransition { .. }));
    }

    #[test]
    fn set_status_accepts_self_transition() {
        let mut store = Store::<TestKind>::new();
        store.insert(item("t-1", TestStatus::Done, vec![])).unwrap();
        // Done → Done is a no-op, allowed even though Done is terminal.
        assert!(store.set_status("t-1", TestStatus::Done).is_ok());
    }

    #[test]
    fn set_status_accepts_legal_transition() {
        let mut store = Store::<TestKind>::new();
        store
            .insert(item("t-1", TestStatus::Active, vec![]))
            .unwrap();
        assert!(store.set_status("t-1", TestStatus::Done).is_ok());
        assert_eq!(store.get("t-1").unwrap().status, TestStatus::Done);
    }

    // --- cross-store cascade ---

    #[test]
    fn cross_kind_cascade_defeats_dependents_when_parent_terminal() {
        let mut parents = Store::<TestParentKind>::new();
        parents
            .insert(parent_item("i-1", TestParentStatus::Active))
            .unwrap();
        parents
            .insert(parent_item("i-2", TestParentStatus::Active))
            .unwrap();

        let mut children = Store::<TestKind>::new();
        children
            .insert(item(
                "t-1",
                TestStatus::Active,
                vec![Justification::ServesIntent {
                    intent_id: "i-1".into(),
                }],
            ))
            .unwrap();
        children
            .insert(item(
                "t-2",
                TestStatus::Active,
                vec![Justification::ServesIntent {
                    intent_id: "i-2".into(),
                }],
            ))
            .unwrap();
        children
            .insert(item(
                "t-3",
                TestStatus::Active,
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

        // Defeat i-1 in parents, then run cascade.
        parents.defeat("i-1", TestParentStatus::Defeated).unwrap();
        let cascaded = cascade_serves_intent(&parents, &mut children, TestStatus::Defeated);

        // t-1 cascade-defeated; t-2 untouched (parent still active);
        // t-3 untouched (one parent still active).
        assert_eq!(cascaded, vec!["t-1".to_string()]);
        assert_eq!(children.get("t-1").unwrap().status, TestStatus::Defeated);
        assert_eq!(
            children.get("t-1").unwrap().defeated_by,
            Some(DefeatedBy::Cascade)
        );
        assert_eq!(children.get("t-2").unwrap().status, TestStatus::Active);
        assert_eq!(children.get("t-3").unwrap().status, TestStatus::Active);

        // Defeat i-2 too; cascade now picks up t-2 and t-3.
        parents.defeat("i-2", TestParentStatus::Defeated).unwrap();
        let cascaded = cascade_serves_intent(&parents, &mut children, TestStatus::Defeated);
        assert!(cascaded.contains(&"t-2".to_string()));
        assert!(cascaded.contains(&"t-3".to_string()));
    }

    #[test]
    fn cross_kind_cascade_ignores_items_referencing_unknown_parent_ids() {
        let parents = Store::<TestParentKind>::new(); // empty
        let mut children = Store::<TestKind>::new();
        children
            .insert(item(
                "t-1",
                TestStatus::Active,
                vec![Justification::ServesIntent {
                    intent_id: "ghost".into(),
                }],
            ))
            .unwrap();
        let cascaded = cascade_serves_intent(&parents, &mut children, TestStatus::Defeated);
        assert!(cascaded.is_empty());
        assert_eq!(children.get("t-1").unwrap().status, TestStatus::Active);
    }

    #[test]
    fn cross_kind_cascade_skips_items_already_terminal() {
        let mut parents = Store::<TestParentKind>::new();
        parents
            .insert(parent_item("i-1", TestParentStatus::Defeated))
            .unwrap();
        let mut children = Store::<TestKind>::new();
        children
            .insert(item(
                "t-1",
                TestStatus::Done, // already terminal
                vec![Justification::ServesIntent {
                    intent_id: "i-1".into(),
                }],
            ))
            .unwrap();
        let cascaded = cascade_serves_intent(&parents, &mut children, TestStatus::Defeated);
        assert!(cascaded.is_empty());
        assert_eq!(children.get("t-1").unwrap().status, TestStatus::Done);
    }
}
