//! Typed schema for `<plan>/memory.yaml`.
//!
//! Memory entries are TMS-shaped items in the `knowledge-graph`
//! substrate. The on-disk record wraps `Item<MemoryEntryKind>` with
//! one memory-specific extension: `attribution` names the target
//! component for promotion at plan finish (entries without
//! attribution are plan-process entries that never promote).
//!
//! See `docs/architecture-next.md` §Item shape and §Continuous
//! attribution for the rationale.

use knowledge_graph::Item;
use serde::{Deserialize, Serialize};

use crate::plan_kg::MemoryEntryKind;

pub const MEMORY_SCHEMA_VERSION: u32 = 1;

/// One on-disk memory entry. The flattened `item` field carries every
/// substrate-defined slot (id, kind marker, claim, justifications,
/// status, supersession links, defeat info, provenance); `attribution`
/// is the only memory-specific extension at v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryEntry {
    #[serde(flatten)]
    pub item: Item<MemoryEntryKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
}

/// The full `memory.yaml` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFile {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<MemoryEntry>,
}

impl Default for MemoryFile {
    fn default() -> Self {
        Self {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_kg::MemoryStatus;
    use knowledge_graph::{Item, Justification, KindMarker};

    fn sample_entry(id: &str, claim: &str, rationale: &str) -> MemoryEntry {
        MemoryEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: claim.into(),
                justifications: vec![Justification::Rationale {
                    text: rationale.into(),
                }],
                status: MemoryStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            attribution: None,
        }
    }

    #[test]
    fn entry_round_trips_through_yaml() {
        let entry = sample_entry("example", "Example claim", "Line one.\n");
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let decoded: MemoryEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, entry);
    }

    #[test]
    fn entry_yaml_carries_kind_string() {
        let entry = sample_entry("ex", "Example", "Body.\n");
        let yaml = serde_yaml::to_string(&entry).unwrap();
        assert!(yaml.contains("kind: memory-entry"), "yaml was:\n{yaml}");
    }

    #[test]
    fn entry_with_attribution_round_trips() {
        let mut entry = sample_entry("ex", "Example", "Body.\n");
        entry.attribution = Some("atlas:atlas-core".into());
        let yaml = serde_yaml::to_string(&entry).unwrap();
        assert!(yaml.contains("attribution: atlas:atlas-core"));
        let decoded: MemoryEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.attribution.as_deref(), Some("atlas:atlas-core"));
    }

    #[test]
    fn entry_without_attribution_omits_field_in_yaml() {
        let entry = sample_entry("ex", "Example", "Body.\n");
        let yaml = serde_yaml::to_string(&entry).unwrap();
        assert!(
            !yaml.contains("attribution"),
            "absent attribution must be skipped on serialise: {yaml}"
        );
    }

    #[test]
    fn memory_file_default_has_current_schema_version() {
        let file = MemoryFile::default();
        assert_eq!(file.schema_version, MEMORY_SCHEMA_VERSION);
        assert!(file.items.is_empty());
    }

    #[test]
    fn memory_file_round_trips_through_yaml() {
        let file = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![sample_entry("a", "Claim A", "Body A.\n")],
        };
        let yaml = serde_yaml::to_string(&file).unwrap();
        let decoded: MemoryFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.schema_version, MEMORY_SCHEMA_VERSION);
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items[0].item.id, "a");
        assert_eq!(decoded.items[0].item.claim, "Claim A");
    }

    #[test]
    fn memory_file_rejects_yaml_without_schema_version() {
        let yaml = "items: []\n";
        let result: Result<MemoryFile, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "schema_version is required; missing must fail"
        );
    }
}
