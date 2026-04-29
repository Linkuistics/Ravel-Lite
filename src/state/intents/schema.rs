//! Typed schema for `<plan>/intents.yaml`.
//!
//! Intent items are TMS-shaped items in the `knowledge-graph`
//! substrate. The on-disk record wraps `Item<IntentKind>` with no
//! intent-specific extension fields at v1; the wrapper exists for
//! shape-parity with `BacklogEntry` and `MemoryEntry` so future
//! extensions (e.g. external-issue links beyond a `Justification::External`)
//! can be added without changing the wire type.
//!
//! See `docs/architecture-next.md` §Plan as a knowledge graph and
//! §Status vocabularies for the rationale.

use knowledge_graph::Item;
use serde::{Deserialize, Serialize};

use crate::plan_kg::IntentKind;

pub const INTENTS_SCHEMA_VERSION: u32 = 1;

/// One on-disk intent entry. The flattened `item` field carries every
/// substrate-defined slot (id, kind marker, claim, justifications,
/// status, supersession links, defeat info, provenance). No
/// intent-specific extensions at v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntentEntry {
    #[serde(flatten)]
    pub item: Item<IntentKind>,
}

/// The full `intents.yaml` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<IntentEntry>,
}

impl Default for IntentsFile {
    fn default() -> Self {
        Self {
            schema_version: INTENTS_SCHEMA_VERSION,
            items: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_kg::IntentStatus;
    use knowledge_graph::{Item, Justification, KindMarker};

    fn sample_entry(id: &str, claim: &str, rationale: &str) -> IntentEntry {
        IntentEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: claim.into(),
                justifications: vec![Justification::Rationale {
                    text: rationale.into(),
                }],
                status: IntentStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
        }
    }

    #[test]
    fn entry_round_trips_through_yaml() {
        let entry = sample_entry("example", "Example claim", "Line one.\n");
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let decoded: IntentEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, entry);
    }

    #[test]
    fn entry_yaml_carries_kind_string() {
        let entry = sample_entry("ex", "Example", "Body.\n");
        let yaml = serde_yaml::to_string(&entry).unwrap();
        assert!(yaml.contains("kind: intent"), "yaml was:\n{yaml}");
    }

    #[test]
    fn entry_yaml_rejects_wrong_kind_string() {
        let yaml = "id: i-1\nkind: memory-entry\nclaim: c\nstatus: active\nauthored_at: t\nauthored_in: x\n";
        let result: Result<IntentEntry, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "expected kind mismatch to fail");
    }

    #[test]
    fn intents_file_default_has_current_schema_version() {
        let file = IntentsFile::default();
        assert_eq!(file.schema_version, INTENTS_SCHEMA_VERSION);
        assert!(file.items.is_empty());
    }

    #[test]
    fn intents_file_round_trips_through_yaml() {
        let file = IntentsFile {
            schema_version: INTENTS_SCHEMA_VERSION,
            items: vec![sample_entry("a", "Claim A", "Body A.\n")],
        };
        let yaml = serde_yaml::to_string(&file).unwrap();
        let decoded: IntentsFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.schema_version, INTENTS_SCHEMA_VERSION);
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items[0].item.id, "a");
        assert_eq!(decoded.items[0].item.claim, "Claim A");
    }

    #[test]
    fn intents_file_rejects_yaml_without_schema_version() {
        let yaml = "items: []\n";
        let result: Result<IntentsFile, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "schema_version is required; missing must fail"
        );
    }
}
