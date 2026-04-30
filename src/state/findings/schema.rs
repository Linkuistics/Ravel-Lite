//! Typed schema for `<context>/findings.yaml`.
//!
//! Findings are TMS-shaped items in the `knowledge-graph` substrate.
//! The on-disk record wraps `Item<FindingKind>` with two finding-
//! specific extensions: `component` carries an optional component-ref
//! attribution, `raised_in` carries an optional plan reference for
//! the plan that surfaced the finding.
//!
//! See `docs/architecture-next.md` §Findings inbox for the rationale.

use knowledge_graph::Item;
use serde::{Deserialize, Serialize};

use crate::plan_kg::FindingKind;

pub const FINDINGS_SCHEMA_VERSION: u32 = 1;

/// One on-disk finding entry. The flattened `item` field carries every
/// substrate-defined slot (id, kind marker, claim, justifications,
/// status, supersession links, defeat info, provenance). `component`
/// and `raised_in` are the finding-specific extensions at v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindingEntry {
    #[serde(flatten)]
    pub item: Item<FindingKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raised_in: Option<String>,
}

/// The full `findings.yaml` document. Uses `items:` to mirror the
/// other TMS files (`intents.yaml`, `backlog.yaml`, `memory.yaml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<FindingEntry>,
}

impl Default for FindingsFile {
    fn default() -> Self {
        Self {
            schema_version: FINDINGS_SCHEMA_VERSION,
            items: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_kg::FindingStatus;
    use knowledge_graph::{Item, Justification, KindMarker};

    fn sample_entry(id: &str, claim: &str, rationale: &str) -> FindingEntry {
        FindingEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: claim.into(),
                justifications: vec![Justification::Rationale {
                    text: rationale.into(),
                }],
                status: FindingStatus::New,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-30T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            component: None,
            raised_in: None,
        }
    }

    #[test]
    fn entry_round_trips_through_yaml() {
        let entry = sample_entry("example", "Example claim", "Line one.\n");
        let yaml = serde_yaml::to_string(&entry).unwrap();
        let decoded: FindingEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, entry);
    }

    #[test]
    fn entry_yaml_carries_kind_string() {
        let entry = sample_entry("ex", "Example", "Body.\n");
        let yaml = serde_yaml::to_string(&entry).unwrap();
        assert!(yaml.contains("kind: finding"), "yaml was:\n{yaml}");
    }

    #[test]
    fn entry_yaml_rejects_wrong_kind_string() {
        let yaml = "id: f-1\nkind: memory-entry\nclaim: c\nstatus: new\nauthored_at: t\nauthored_in: x\n";
        let result: Result<FindingEntry, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "expected kind mismatch to fail");
    }

    #[test]
    fn entry_with_component_round_trips() {
        let mut entry = sample_entry("ex", "Example", "Body.\n");
        entry.component = Some("atlas:atlas-ontology".into());
        let yaml = serde_yaml::to_string(&entry).unwrap();
        assert!(yaml.contains("component: atlas:atlas-ontology"));
        let decoded: FindingEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.component.as_deref(), Some("atlas:atlas-ontology"));
    }

    #[test]
    fn entry_with_raised_in_round_trips() {
        let mut entry = sample_entry("ex", "Example", "Body.\n");
        entry.raised_in = Some("plan/foo".into());
        let yaml = serde_yaml::to_string(&entry).unwrap();
        assert!(yaml.contains("raised_in: plan/foo"));
        let decoded: FindingEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.raised_in.as_deref(), Some("plan/foo"));
    }

    #[test]
    fn entry_without_extensions_omits_fields_in_yaml() {
        let entry = sample_entry("ex", "Example", "Body.\n");
        let yaml = serde_yaml::to_string(&entry).unwrap();
        assert!(
            !yaml.contains("component"),
            "absent component must be skipped on serialise: {yaml}"
        );
        assert!(
            !yaml.contains("raised_in"),
            "absent raised_in must be skipped on serialise: {yaml}"
        );
    }

    #[test]
    fn findings_file_default_has_current_schema_version() {
        let file = FindingsFile::default();
        assert_eq!(file.schema_version, FINDINGS_SCHEMA_VERSION);
        assert!(file.items.is_empty());
    }

    #[test]
    fn findings_file_round_trips_through_yaml() {
        let file = FindingsFile {
            schema_version: FINDINGS_SCHEMA_VERSION,
            items: vec![sample_entry("a", "Claim A", "Body A.\n")],
        };
        let yaml = serde_yaml::to_string(&file).unwrap();
        let decoded: FindingsFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.schema_version, FINDINGS_SCHEMA_VERSION);
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items[0].item.id, "a");
        assert_eq!(decoded.items[0].item.claim, "Claim A");
    }

    #[test]
    fn findings_file_rejects_yaml_without_schema_version() {
        let yaml = "items: []\n";
        let result: Result<FindingsFile, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "schema_version is required; missing must fail"
        );
    }
}
