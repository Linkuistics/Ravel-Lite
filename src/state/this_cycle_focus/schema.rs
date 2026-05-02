//! Typed schema for `<plan>/this-cycle-focus.yaml`.
//!
//! `this-cycle-focus.yaml` is a single-document scratch file written by
//! the triage phase and consumed by the work phase: it names the target
//! component the cycle is focused on, lists the backlog item ids work
//! should attempt, and carries optional human-readable notes (e.g.
//! ordering, deferrals, escalation reasons). See
//! `docs/architecture-next.md` §TRIAGE step 6 (Focus selection) and
//! §WORK for the contract.
//!
//! The file is one-shot: triage writes it at the start of every cycle
//! and the work phase reads it; analyse-work and reflect ignore it; the
//! file's lifetime is bounded to a single cycle. Unlike the TMS state
//! files (`intents.yaml`, `backlog.yaml`, `memory.yaml`), the focus
//! record is not a knowledge-graph item — it is a partitioning
//! decision document, similar in shape to `target-requests.yaml`.
//!
//! `target` is a ComponentRef in `<repo_slug>:<component_id>` notation,
//! matching the rest of the v2 surface. v1 supports a single target
//! per cycle; the multi-target escape hatch documented in the
//! architecture doc is a future extension.

use serde::{Deserialize, Serialize};

use crate::component_ref::ComponentRef;

pub const THIS_CYCLE_FOCUS_SCHEMA_VERSION: u32 = 1;

/// One cycle's focus record. `target` is a typed `ComponentRef`; its
/// serde impl emits the `<repo_slug>:<component_id>` string scalar on
/// the wire, matching the rest of the v2 surface. `backlog_items` lists
/// the backlog item ids work should attempt; an empty list is valid (a
/// "look around, no specific items" cycle). `notes` is free-form prose
/// surfaced verbatim into the work-phase prompt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThisCycleFocus {
    pub schema_version: u32,
    pub target: ComponentRef,
    #[serde(default)]
    pub backlog_items: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl ThisCycleFocus {
    /// Build a focus record at the current schema version with no
    /// backlog items and no notes — useful as a starting point when the
    /// caller intends to mutate the record before writing.
    pub fn new(target: ComponentRef) -> Self {
        Self {
            schema_version: THIS_CYCLE_FOCUS_SCHEMA_VERSION,
            target,
            backlog_items: Vec::new(),
            notes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_record_has_current_schema_version_and_empty_collections() {
        let focus = ThisCycleFocus::new(ComponentRef::new("atlas", "atlas-core"));
        assert_eq!(focus.schema_version, THIS_CYCLE_FOCUS_SCHEMA_VERSION);
        assert_eq!(focus.target, ComponentRef::new("atlas", "atlas-core"));
        assert!(focus.backlog_items.is_empty());
        assert!(focus.notes.is_none());
    }

    #[test]
    fn full_record_round_trips_through_yaml() {
        let focus = ThisCycleFocus {
            schema_version: THIS_CYCLE_FOCUS_SCHEMA_VERSION,
            target: ComponentRef::new("atlas", "atlas-core"),
            backlog_items: vec!["t-001".into(), "t-005".into()],
            notes: Some("t-005 depends on t-001 — do them in order.\n".into()),
        };
        let yaml = serde_yaml::to_string(&focus).unwrap();
        let decoded: ThisCycleFocus = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, focus);
    }

    #[test]
    fn target_serialises_as_string_scalar() {
        // Wire-shape compatibility: typed `ComponentRef` field still
        // emits a single string scalar (`target: <repo>:<id>`), matching
        // the v1 wire shape so existing on-disk files keep round-tripping.
        let focus = ThisCycleFocus::new(ComponentRef::new("atlas", "atlas-core"));
        let yaml = serde_yaml::to_string(&focus).unwrap();
        assert!(
            yaml.contains("target: atlas:atlas-core"),
            "target must serialise as `<repo>:<id>` scalar: {yaml}"
        );
    }

    #[test]
    fn missing_backlog_items_defaults_to_empty() {
        let yaml = "schema_version: 1\ntarget: atlas:atlas-core\n";
        let parsed: ThisCycleFocus = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.backlog_items.is_empty());
        assert!(parsed.notes.is_none());
    }

    #[test]
    fn missing_schema_version_fails_to_parse() {
        let yaml = "target: atlas:atlas-core\nbacklog_items: []\n";
        let result: Result<ThisCycleFocus, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "schema_version is required; missing must fail"
        );
    }

    #[test]
    fn missing_target_fails_to_parse() {
        let yaml = "schema_version: 1\nbacklog_items: []\n";
        let result: Result<ThisCycleFocus, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "target is required; missing must fail");
    }

    #[test]
    fn absent_notes_does_not_serialise_a_null() {
        // `notes: None` should not emit `notes: null` — that round-trips,
        // but the cleaner wire shape is to omit the key entirely.
        let focus = ThisCycleFocus::new(ComponentRef::new("atlas", "atlas-core"));
        let yaml = serde_yaml::to_string(&focus).unwrap();
        assert!(
            !yaml.contains("notes:"),
            "notes key should be omitted when None: {yaml}"
        );
    }
}
