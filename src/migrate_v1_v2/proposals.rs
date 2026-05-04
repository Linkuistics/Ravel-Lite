//! Scratch-file schemas the three migrate-* phases write.
//!
//! Each Migrate* phase emits one of these YAML documents into the new
//! plan dir. The runner reads, validates, applies, and deletes the
//! scratch file.

use serde::{Deserialize, Serialize};

use crate::state::intents::IntentEntry;

pub const INTENT_PROPOSAL_FILENAME: &str = "migrate-intent-proposal.yaml";
pub const TARGETS_PROPOSAL_FILENAME: &str = "migrate-targets-proposal.yaml";
pub const MEMORY_PROPOSAL_FILENAME: &str = "migrate-memory-proposal.yaml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentProposal {
    pub intents: Vec<IntentEntry>,
    #[serde(default)]
    pub item_attributions: Vec<ItemAttribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemAttribution {
    pub item_id: String,
    /// Either an intent id from `intents` above, or the literal "legacy".
    pub serves: String,
}

impl ItemAttribution {
    pub fn is_legacy(&self) -> bool {
        self.serves == "legacy"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetsProposal {
    pub targets: Vec<TargetProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetProposal {
    /// Atlas component id within the source repo.
    pub component_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProposal {
    pub attributions: Vec<MemoryAttribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryAttribution {
    pub entry_id: String,
    /// `<repo_slug>:<component_id>`, the literal "plan-process", or null.
    /// Null entries also receive `status: legacy` on apply.
    #[serde(default)]
    pub attribution: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_proposal_round_trips() {
        let p = IntentProposal {
            intents: vec![],
            item_attributions: vec![ItemAttribution {
                item_id: "t-001".into(),
                serves: "i-001".into(),
            }],
        };
        let yaml = serde_yaml::to_string(&p).unwrap();
        let decoded: IntentProposal = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.item_attributions.len(), 1);
        assert_eq!(decoded.item_attributions[0].item_id, "t-001");
    }

    #[test]
    fn item_attribution_is_legacy_recognises_literal() {
        let a = ItemAttribution {
            item_id: "t".into(),
            serves: "legacy".into(),
        };
        assert!(a.is_legacy());
        let b = ItemAttribution {
            item_id: "t".into(),
            serves: "i-001".into(),
        };
        assert!(!b.is_legacy());
    }

    #[test]
    fn targets_proposal_round_trips() {
        let p = TargetsProposal {
            targets: vec![TargetProposal {
                component_id: "core".into(),
            }],
        };
        let yaml = serde_yaml::to_string(&p).unwrap();
        let decoded: TargetsProposal = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.targets.len(), 1);
        assert_eq!(decoded.targets[0].component_id, "core");
    }

    #[test]
    fn memory_proposal_round_trips_with_null_attribution() {
        let p = MemoryProposal {
            attributions: vec![
                MemoryAttribution {
                    entry_id: "m-001".into(),
                    attribution: Some("atlas:atlas-core".into()),
                },
                MemoryAttribution {
                    entry_id: "m-002".into(),
                    attribution: None,
                },
            ],
        };
        let yaml = serde_yaml::to_string(&p).unwrap();
        let decoded: MemoryProposal = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.attributions.len(), 2);
        assert!(decoded.attributions[1].attribution.is_none());
    }
}
