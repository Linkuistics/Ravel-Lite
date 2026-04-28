use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::justification::Justification;

pub type ItemId = String;

const CASCADE_MARKER: &str = "cascade";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefeatedBy {
    Cascade,
    Item(ItemId),
}

impl Serialize for DefeatedBy {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            DefeatedBy::Cascade => s.serialize_str(CASCADE_MARKER),
            DefeatedBy::Item(id) => s.serialize_str(id),
        }
    }
}

impl<'de> Deserialize<'de> for DefeatedBy {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(if s == CASCADE_MARKER {
            DefeatedBy::Cascade
        } else {
            DefeatedBy::Item(s)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub kind: String,
    pub claim: String,
    #[serde(default)]
    pub justifications: Vec<Justification>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<ItemId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<ItemId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defeated_by: Option<DefeatedBy>,
    pub authored_at: String,
    pub authored_in: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_roundtrip_minimal() {
        let item = Item {
            id: "t-001".into(),
            kind: "backlog-item".into(),
            claim: "Wire plan KG kinds".into(),
            justifications: vec![],
            status: "active".into(),
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: "2026-04-28T10:00:00Z".into(),
            authored_in: "create".into(),
        };
        let yaml = serde_yaml::to_string(&item).unwrap();
        let back: Item = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(item, back);
    }

    #[test]
    fn defeated_by_cascade_roundtrips_as_string() {
        let yaml = "id: t-1\nkind: backlog-item\nclaim: c\nstatus: defeated\ndefeated_by: cascade\nauthored_at: t\nauthored_in: triage\n";
        let item: Item = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(item.defeated_by, Some(DefeatedBy::Cascade));
        let back = serde_yaml::to_string(&item).unwrap();
        assert!(back.contains("defeated_by: cascade"));
    }

    #[test]
    fn defeated_by_item_id_roundtrips_as_string() {
        let yaml = "id: t-2\nkind: backlog-item\nclaim: c\nstatus: defeated\ndefeated_by: i-007\nauthored_at: t\nauthored_in: triage\n";
        let item: Item = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(item.defeated_by, Some(DefeatedBy::Item("i-007".into())));
        let back = serde_yaml::to_string(&item).unwrap();
        assert!(back.contains("defeated_by: i-007"));
    }
}
