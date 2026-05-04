//! Plan-side TMS vocabulary: kind tag types and status enums.
//!
//! The `knowledge-graph` substrate is generic over an `ItemKind` trait
//! and an `ItemStatus` trait. This module is the consumer-side
//! registration of ravel-lite's four plan kinds — intent, backlog
//! item, memory entry, finding — together with their per-kind status
//! enums and transition tables.
//!
//! There are no runtime `validate_kind`/`validate_status` functions:
//! `Item<IntentKind>` cannot carry a backlog status by construction,
//! and `Store<IntentKind>::set_status` enforces transition rules
//! against the typed table. Type-level enforcement replaces what
//! would otherwise have been runtime string validation.
//!
//! Status vocabularies follow `docs/architecture-next.md` §Status
//! vocabularies:
//!
//! - **Intent:** `active | satisfied | defeated | superseded`
//! - **Backlog item:** `active | done | defeated | superseded | blocked`
//! - **Memory entry:** `active | defeated | superseded`
//! - **Finding:** `new | promoted | wontfix | superseded`

use knowledge_graph::{ItemKind, ItemStatus};
use serde::{Deserialize, Serialize};

// ----- Intent ----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IntentStatus {
    Active,
    Satisfied,
    Defeated,
    Superseded,
}

impl ItemStatus for IntentStatus {
    fn as_str(self) -> &'static str {
        match self {
            IntentStatus::Active => "active",
            IntentStatus::Satisfied => "satisfied",
            IntentStatus::Defeated => "defeated",
            IntentStatus::Superseded => "superseded",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(IntentStatus::Active),
            "satisfied" => Some(IntentStatus::Satisfied),
            "defeated" => Some(IntentStatus::Defeated),
            "superseded" => Some(IntentStatus::Superseded),
            _ => None,
        }
    }
    fn is_terminal(self) -> bool {
        !matches!(self, IntentStatus::Active)
    }
    fn transitions() -> &'static [(Self, Self)] {
        &[
            (IntentStatus::Active, IntentStatus::Satisfied),
            (IntentStatus::Active, IntentStatus::Defeated),
            (IntentStatus::Active, IntentStatus::Superseded),
        ]
    }
}

pub struct IntentKind;
impl ItemKind for IntentKind {
    type Status = IntentStatus;
    const KIND_STR: &'static str = "intent";
}

// ----- Backlog item ----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BacklogStatus {
    Active,
    Done,
    Defeated,
    Superseded,
    Blocked,
}

impl ItemStatus for BacklogStatus {
    fn as_str(self) -> &'static str {
        match self {
            BacklogStatus::Active => "active",
            BacklogStatus::Done => "done",
            BacklogStatus::Defeated => "defeated",
            BacklogStatus::Superseded => "superseded",
            BacklogStatus::Blocked => "blocked",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(BacklogStatus::Active),
            "done" => Some(BacklogStatus::Done),
            "defeated" => Some(BacklogStatus::Defeated),
            "superseded" => Some(BacklogStatus::Superseded),
            "blocked" => Some(BacklogStatus::Blocked),
            _ => None,
        }
    }
    fn is_terminal(self) -> bool {
        !matches!(self, BacklogStatus::Active | BacklogStatus::Blocked)
    }
    fn transitions() -> &'static [(Self, Self)] {
        &[
            (BacklogStatus::Active, BacklogStatus::Done),
            (BacklogStatus::Active, BacklogStatus::Defeated),
            (BacklogStatus::Active, BacklogStatus::Superseded),
            (BacklogStatus::Active, BacklogStatus::Blocked),
            (BacklogStatus::Blocked, BacklogStatus::Active),
            (BacklogStatus::Blocked, BacklogStatus::Done),
            (BacklogStatus::Blocked, BacklogStatus::Defeated),
            (BacklogStatus::Blocked, BacklogStatus::Superseded),
        ]
    }
}

pub struct BacklogItemKind;
impl ItemKind for BacklogItemKind {
    type Status = BacklogStatus;
    const KIND_STR: &'static str = "backlog-item";
}

// ----- Memory entry ----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MemoryStatus {
    Active,
    Defeated,
    Superseded,
    /// Migrated from a v1 plan but the migrate-memory-backfill phase
    /// could not attribute the entry to any component. Awaits user
    /// curation: either re-attribute (→ Active) or defeat.
    Legacy,
}

impl ItemStatus for MemoryStatus {
    fn as_str(self) -> &'static str {
        match self {
            MemoryStatus::Active => "active",
            MemoryStatus::Defeated => "defeated",
            MemoryStatus::Superseded => "superseded",
            MemoryStatus::Legacy => "legacy",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(MemoryStatus::Active),
            "defeated" => Some(MemoryStatus::Defeated),
            "superseded" => Some(MemoryStatus::Superseded),
            "legacy" => Some(MemoryStatus::Legacy),
            _ => None,
        }
    }
    fn is_terminal(self) -> bool {
        matches!(self, MemoryStatus::Defeated | MemoryStatus::Superseded)
    }
    fn transitions() -> &'static [(Self, Self)] {
        &[
            (MemoryStatus::Active, MemoryStatus::Defeated),
            (MemoryStatus::Active, MemoryStatus::Superseded),
            // Legacy is awaiting curation: the user can either bring it
            // back to active (after attributing) or defeat it.
            (MemoryStatus::Legacy, MemoryStatus::Active),
            (MemoryStatus::Legacy, MemoryStatus::Defeated),
        ]
    }
}

pub struct MemoryEntryKind;
impl ItemKind for MemoryEntryKind {
    type Status = MemoryStatus;
    const KIND_STR: &'static str = "memory-entry";
}

// ----- Finding ---------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingStatus {
    New,
    Promoted,
    Wontfix,
    Superseded,
}

impl ItemStatus for FindingStatus {
    fn as_str(self) -> &'static str {
        match self {
            FindingStatus::New => "new",
            FindingStatus::Promoted => "promoted",
            FindingStatus::Wontfix => "wontfix",
            FindingStatus::Superseded => "superseded",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "new" => Some(FindingStatus::New),
            "promoted" => Some(FindingStatus::Promoted),
            "wontfix" => Some(FindingStatus::Wontfix),
            "superseded" => Some(FindingStatus::Superseded),
            _ => None,
        }
    }
    fn is_terminal(self) -> bool {
        !matches!(self, FindingStatus::New)
    }
    fn transitions() -> &'static [(Self, Self)] {
        &[
            (FindingStatus::New, FindingStatus::Promoted),
            (FindingStatus::New, FindingStatus::Wontfix),
            (FindingStatus::New, FindingStatus::Superseded),
        ]
    }
}

pub struct FindingKind;
impl ItemKind for FindingKind {
    type Status = FindingStatus;
    const KIND_STR: &'static str = "finding";
}

// Type aliases for the four typed item/store variants. Callers should
// reach for these names rather than spelling out `Item<IntentKind>`.
pub type IntentItem = knowledge_graph::Item<IntentKind>;
pub type BacklogItemItem = knowledge_graph::Item<BacklogItemKind>;
pub type MemoryItem = knowledge_graph::Item<MemoryEntryKind>;
pub type FindingItem = knowledge_graph::Item<FindingKind>;

pub type IntentStore = knowledge_graph::Store<IntentKind>;
pub type BacklogStore = knowledge_graph::Store<BacklogItemKind>;
pub type MemoryStore = knowledge_graph::Store<MemoryEntryKind>;
pub type FindingStore = knowledge_graph::Store<FindingKind>;

#[cfg(test)]
mod tests {
    use super::*;
    use knowledge_graph::{Item, ItemStatus, KindMarker};

    fn intent_item(id: &str, status: IntentStatus) -> IntentItem {
        Item {
            id: id.into(),
            kind: KindMarker::new(),
            claim: format!("intent {id}"),
            justifications: vec![],
            status,
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: "2026-04-29T00:00:00Z".into(),
            authored_in: "test".into(),
        }
    }

    #[test]
    fn intent_status_yaml_round_trip() {
        for s in [
            IntentStatus::Active,
            IntentStatus::Satisfied,
            IntentStatus::Defeated,
            IntentStatus::Superseded,
        ] {
            let yaml = serde_yaml::to_string(&s).unwrap();
            let back: IntentStatus = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn backlog_status_yaml_round_trip() {
        for s in [
            BacklogStatus::Active,
            BacklogStatus::Done,
            BacklogStatus::Defeated,
            BacklogStatus::Superseded,
            BacklogStatus::Blocked,
        ] {
            let yaml = serde_yaml::to_string(&s).unwrap();
            let back: BacklogStatus = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn memory_status_yaml_round_trip() {
        for s in [
            MemoryStatus::Active,
            MemoryStatus::Defeated,
            MemoryStatus::Superseded,
            MemoryStatus::Legacy,
        ] {
            let yaml = serde_yaml::to_string(&s).unwrap();
            let back: MemoryStatus = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn finding_status_yaml_round_trip() {
        for s in [
            FindingStatus::New,
            FindingStatus::Promoted,
            FindingStatus::Wontfix,
            FindingStatus::Superseded,
        ] {
            let yaml = serde_yaml::to_string(&s).unwrap();
            let back: FindingStatus = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn kind_strings_match_doc() {
        assert_eq!(IntentKind::KIND_STR, "intent");
        assert_eq!(BacklogItemKind::KIND_STR, "backlog-item");
        assert_eq!(MemoryEntryKind::KIND_STR, "memory-entry");
        assert_eq!(FindingKind::KIND_STR, "finding");
    }

    #[test]
    fn terminal_states_match_doc() {
        assert!(!IntentStatus::Active.is_terminal());
        assert!(IntentStatus::Satisfied.is_terminal());

        assert!(!BacklogStatus::Active.is_terminal());
        assert!(!BacklogStatus::Blocked.is_terminal());
        assert!(BacklogStatus::Done.is_terminal());

        assert!(!MemoryStatus::Active.is_terminal());
        assert!(MemoryStatus::Defeated.is_terminal());

        assert!(!FindingStatus::New.is_terminal());
        assert!(FindingStatus::Promoted.is_terminal());
    }

    #[test]
    fn transition_tables_only_originate_from_non_terminal_states() {
        // The doc says terminal items don't transition out; the table
        // should reflect that by construction. A regression here means
        // someone accidentally added an outgoing edge from a terminal.
        for (f, _) in IntentStatus::transitions() {
            assert!(!f.is_terminal(), "intent terminal {f:?} has outgoing edge");
        }
        for (f, _) in BacklogStatus::transitions() {
            assert!(!f.is_terminal(), "backlog terminal {f:?} has outgoing edge");
        }
        for (f, _) in MemoryStatus::transitions() {
            assert!(!f.is_terminal(), "memory terminal {f:?} has outgoing edge");
        }
        for (f, _) in FindingStatus::transitions() {
            assert!(!f.is_terminal(), "finding terminal {f:?} has outgoing edge");
        }
    }

    #[test]
    fn store_set_status_enforces_transitions() {
        let mut store = IntentStore::new();
        store
            .insert(intent_item("i-1", IntentStatus::Active))
            .unwrap();
        // Legal: active → satisfied
        assert!(store.set_status("i-1", IntentStatus::Satisfied).is_ok());
        // Illegal: satisfied → active (terminal can't move).
        assert!(store.set_status("i-1", IntentStatus::Active).is_err());
    }

    #[test]
    fn item_yaml_carries_kind_string() {
        let item = intent_item("i-1", IntentStatus::Active);
        let yaml = serde_yaml::to_string(&item).unwrap();
        assert!(yaml.contains("kind: intent"), "yaml was:\n{yaml}");
        let back: IntentItem = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.id, "i-1");
        assert_eq!(back.status, IntentStatus::Active);
    }

    #[test]
    fn item_yaml_rejects_wrong_kind_string() {
        // A YAML carrying `kind: backlog-item` cannot deserialise as an
        // `Item<IntentKind>` — the marker check fails. Type-level kind
        // enforcement at the wire boundary.
        let yaml = "id: t-1\nkind: backlog-item\nclaim: c\nstatus: active\nauthored_at: t\nauthored_in: x\n";
        let result: Result<IntentItem, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "expected kind mismatch to fail");
    }

    #[test]
    fn cross_store_cascade_intent_to_backlog() {
        // The realistic plan-KG case: an intent in `IntentStore` is
        // defeated, and backlog items in `BacklogStore` whose every
        // serves-intent points at defeated intents are cascade-defeated.
        use knowledge_graph::{cascade_serves_intent, Justification};

        let mut intents = IntentStore::new();
        intents
            .insert(intent_item("i-1", IntentStatus::Active))
            .unwrap();

        let mut backlog = BacklogStore::new();
        backlog
            .insert(BacklogItemItem {
                id: "t-1".into(),
                kind: KindMarker::new(),
                claim: "do the thing".into(),
                justifications: vec![Justification::ServesIntent {
                    intent_id: "i-1".into(),
                }],
                status: BacklogStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "t".into(),
                authored_in: "test".into(),
            })
            .unwrap();

        intents.defeat("i-1", IntentStatus::Defeated).unwrap();
        let cascaded = cascade_serves_intent(&intents, &mut backlog, BacklogStatus::Defeated);

        assert_eq!(cascaded, vec!["t-1".to_string()]);
        assert_eq!(backlog.get("t-1").unwrap().status, BacklogStatus::Defeated);
    }
}
