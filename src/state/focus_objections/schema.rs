//! Typed schema for `<plan>/focus-objections.yaml`.
//!
//! `focus-objections.yaml` is the work-phase escalation channel: when
//! triage's selected focus turns out to be wrong, the work phase records
//! its objections in this file rather than silently editing the wrong
//! component. The next triage drains the file at the start of its run.
//!
//! Three objection kinds are recognised:
//!
//! - `wrong-target` — the chosen target is not the right component;
//!   work suggests a replacement.
//! - `skip-item` — a specific backlog item is not ready (blocked, in
//!   flux upstream, etc.) and should be deferred.
//! - `premature` — the whole focus is premature; some prerequisite
//!   work or learning has to happen first.
//!
//! All three carry a free-form `reasoning` field that flows verbatim
//! into the next triage prompt. `wrong-target` adds a typed
//! ComponentRef and `skip-item` a typed backlog id, so the runner can
//! handle the mechanical parts (re-mounting, status updates) without
//! re-parsing prose. The kind discriminator is encoded as
//! `kind: <kebab>` on the wire.
//!
//! See `docs/architecture-next.md` §WORK and §TRIAGE step 1 (Intent
//! hygiene) / step 3 (Backlog hygiene) for how triage consumes each
//! objection kind.

use serde::{Deserialize, Serialize};

use crate::component_ref::ComponentRef;

pub const FOCUS_OBJECTIONS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Objection {
    /// The chosen target is not the right component for this work.
    /// `suggested_target` proposes a replacement; serialises as the
    /// canonical `<repo_slug>:<component_id>` string form.
    WrongTarget {
        suggested_target: ComponentRef,
        reasoning: String,
    },
    /// A specific backlog item is not ready and should be deferred.
    /// `item_id` references the backlog row by id.
    SkipItem { item_id: String, reasoning: String },
    /// The whole focus is premature — some prerequisite must happen
    /// before this work can be attempted.
    Premature { reasoning: String },
}

impl Objection {
    /// Wire-form discriminator for human-readable error messages and
    /// CLI help — matches the `serde(tag = "kind")` strings exactly.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Objection::WrongTarget { .. } => "wrong-target",
            Objection::SkipItem { .. } => "skip-item",
            Objection::Premature { .. } => "premature",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FocusObjectionsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub objections: Vec<Objection>,
}

impl Default for FocusObjectionsFile {
    fn default() -> Self {
        Self {
            schema_version: FOCUS_OBJECTIONS_SCHEMA_VERSION,
            objections: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_target_round_trips_with_kebab_kind() {
        let obj = Objection::WrongTarget {
            suggested_target: ComponentRef::new("atlas", "atlas-ontology"),
            reasoning: "Edit needs ontology-side change first.\n".into(),
        };
        let yaml = serde_yaml::to_string(&obj).unwrap();
        assert!(
            yaml.contains("kind: wrong-target"),
            "kind must be kebab-cased: {yaml}"
        );
        assert!(
            yaml.contains("suggested_target: atlas:atlas-ontology"),
            "suggested_target must serialise as <repo>:<id> string: {yaml}"
        );
        assert_eq!(serde_yaml::from_str::<Objection>(&yaml).unwrap(), obj);
    }

    #[test]
    fn wrong_target_rejects_malformed_suggested_target_at_parse() {
        let yaml = "kind: wrong-target\nsuggested_target: no-colon\nreasoning: x\n";
        let result: Result<Objection, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "ComponentRef must reject missing-colon at parse time"
        );
    }

    #[test]
    fn skip_item_round_trips_with_kebab_kind() {
        let obj = Objection::SkipItem {
            item_id: "t-007".into(),
            reasoning: "Blocked on a refactor in main.\n".into(),
        };
        let yaml = serde_yaml::to_string(&obj).unwrap();
        assert!(yaml.contains("kind: skip-item"), "{yaml}");
        assert_eq!(serde_yaml::from_str::<Objection>(&yaml).unwrap(), obj);
    }

    #[test]
    fn premature_round_trips_with_kebab_kind() {
        let obj = Objection::Premature {
            reasoning: "Need to understand X first.\n".into(),
        };
        let yaml = serde_yaml::to_string(&obj).unwrap();
        assert!(yaml.contains("kind: premature"), "{yaml}");
        assert_eq!(serde_yaml::from_str::<Objection>(&yaml).unwrap(), obj);
    }

    #[test]
    fn unknown_kind_fails_to_parse() {
        let yaml = "kind: invented-kind\nreasoning: x\n";
        let result: Result<Objection, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "unknown kinds must fail: {yaml}");
    }

    #[test]
    fn missing_required_field_fails_to_parse() {
        // wrong-target requires suggested_target
        let yaml = "kind: wrong-target\nreasoning: x\n";
        let result: Result<Objection, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "missing suggested_target must fail");

        // skip-item requires item_id
        let yaml = "kind: skip-item\nreasoning: x\n";
        let result: Result<Objection, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "missing item_id must fail");
    }

    #[test]
    fn kind_str_matches_wire_discriminator() {
        assert_eq!(
            Objection::WrongTarget {
                suggested_target: ComponentRef::new("a", "b"),
                reasoning: "r".into(),
            }
            .kind_str(),
            "wrong-target"
        );
        assert_eq!(
            Objection::SkipItem {
                item_id: "t-1".into(),
                reasoning: "r".into(),
            }
            .kind_str(),
            "skip-item"
        );
        assert_eq!(
            Objection::Premature {
                reasoning: "r".into(),
            }
            .kind_str(),
            "premature"
        );
    }

    #[test]
    fn file_default_has_current_schema_version() {
        let file = FocusObjectionsFile::default();
        assert_eq!(file.schema_version, FOCUS_OBJECTIONS_SCHEMA_VERSION);
        assert!(file.objections.is_empty());
    }

    #[test]
    fn file_round_trips_with_mixed_objection_kinds() {
        let file = FocusObjectionsFile {
            schema_version: FOCUS_OBJECTIONS_SCHEMA_VERSION,
            objections: vec![
                Objection::WrongTarget {
                    suggested_target: ComponentRef::new("atlas", "ontology"),
                    reasoning: "Need ontology change first.\n".into(),
                },
                Objection::SkipItem {
                    item_id: "t-007".into(),
                    reasoning: "Blocked upstream.\n".into(),
                },
                Objection::Premature {
                    reasoning: "Understand X first.\n".into(),
                },
            ],
        };
        let yaml = serde_yaml::to_string(&file).unwrap();
        let decoded: FocusObjectionsFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, file);
    }

    #[test]
    fn file_rejects_yaml_without_schema_version() {
        let yaml = "objections: []\n";
        let result: Result<FocusObjectionsFile, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "schema_version is required");
    }

    #[test]
    fn file_accepts_yaml_without_objections_key() {
        let yaml = "schema_version: 1\n";
        let parsed: FocusObjectionsFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.schema_version, FOCUS_OBJECTIONS_SCHEMA_VERSION);
        assert!(parsed.objections.is_empty());
    }
}
