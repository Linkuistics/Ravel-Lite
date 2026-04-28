use serde::{Deserialize, Serialize};

use crate::item::ItemId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Justification {
    CodeAnchor {
        component: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<String>,
        sha_at_assertion: String,
    },
    Rationale {
        text: String,
    },
    ServesIntent {
        intent_id: ItemId,
    },
    Defeats {
        item_id: ItemId,
    },
    Supersedes {
        item_id: ItemId,
    },
    External {
        uri: String,
    },
}

impl Justification {
    /// Item id this justification points at, if any.
    /// Cascade walks the graph along these references.
    pub fn references_item(&self) -> Option<&ItemId> {
        match self {
            Justification::ServesIntent { intent_id } => Some(intent_id),
            Justification::Defeats { item_id } => Some(item_id),
            Justification::Supersedes { item_id } => Some(item_id),
            Justification::CodeAnchor { .. }
            | Justification::Rationale { .. }
            | Justification::External { .. } => None,
        }
    }

    pub fn is_serves_intent_for(&self, intent_id: &str) -> bool {
        matches!(
            self,
            Justification::ServesIntent { intent_id: i } if i == intent_id
        )
    }

    pub fn is_serves_intent(&self) -> bool {
        matches!(self, Justification::ServesIntent { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_roundtrips_for_each_kind() {
        let cases = vec![
            Justification::CodeAnchor {
                component: "atlas:atlas-core".into(),
                path: "src/lib.rs".into(),
                lines: Some("10-25".into()),
                sha_at_assertion: "abc123".into(),
            },
            Justification::Rationale {
                text: "Conversation with user 2026-04-28".into(),
            },
            Justification::ServesIntent {
                intent_id: "i-001".into(),
            },
            Justification::Defeats {
                item_id: "m-005".into(),
            },
            Justification::Supersedes {
                item_id: "m-006".into(),
            },
            Justification::External {
                uri: "https://github.com/issues/1".into(),
            },
        ];
        for j in cases {
            let yaml = serde_yaml::to_string(&j).unwrap();
            let back: Justification = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(j, back, "roundtrip failed for: {yaml}");
        }
    }

    #[test]
    fn references_item_picks_id_carrying_kinds() {
        let j = Justification::ServesIntent {
            intent_id: "i-1".into(),
        };
        assert_eq!(j.references_item(), Some(&"i-1".to_string()));

        let j = Justification::Rationale { text: "x".into() };
        assert_eq!(j.references_item(), None);
    }
}
