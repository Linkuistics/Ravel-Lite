use std::marker::PhantomData;

use serde::de::DeserializeOwned;
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

/// Status vocabulary for one kind. The substrate stays vocabulary-
/// agnostic by deferring all kind-specific knowledge (legal values,
/// terminal states, transitions) to the consumer's impl. Required
/// `Serialize`/`DeserializeOwned` bounds let `Item<K>` derive its own
/// serde without further plumbing.
pub trait ItemStatus:
    Copy + Eq + std::fmt::Debug + Serialize + DeserializeOwned + 'static
{
    fn as_str(self) -> &'static str;
    fn parse(s: &str) -> Option<Self>;
    fn is_terminal(self) -> bool;
    /// Legal `(from, to)` transition pairs, excluding self-loops.
    fn transitions() -> &'static [(Self, Self)];
}

/// Tag type for a single kind of item. Used as a `K` parameter on
/// `Item<K>` and `Store<K>`. The type itself is typically a unit
/// struct; only its associated `Status` and `KIND_STR` matter to the
/// substrate.
pub trait ItemKind: 'static {
    type Status: ItemStatus;
    /// Canonical kind string, written into the YAML `kind:` field.
    const KIND_STR: &'static str;
}

/// Zero-sized field that owns the YAML round-trip for the `kind:`
/// string. On serialise it writes `K::KIND_STR`; on deserialise it
/// rejects any value that does not match.
///
/// Standard derives (`Clone`, `Debug`, `PartialEq`, `Eq`) are written
/// manually because `#[derive]` would generate overly-restrictive
/// `where K: Clone` bounds even though `PhantomData<K>` is unconditional.
pub struct KindMarker<K: ItemKind>(PhantomData<K>);

impl<K: ItemKind> KindMarker<K> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<K: ItemKind> Default for KindMarker<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: ItemKind> Clone for KindMarker<K> {
    fn clone(&self) -> Self {
        Self::new()
    }
}

impl<K: ItemKind> Copy for KindMarker<K> {}

impl<K: ItemKind> std::fmt::Debug for KindMarker<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KindMarker({})", K::KIND_STR)
    }
}

impl<K: ItemKind> PartialEq for KindMarker<K> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<K: ItemKind> Eq for KindMarker<K> {}

impl<K: ItemKind> Serialize for KindMarker<K> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(K::KIND_STR)
    }
}

impl<'de, K: ItemKind> Deserialize<'de> for KindMarker<K> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == K::KIND_STR {
            Ok(KindMarker(PhantomData))
        } else {
            Err(serde::de::Error::custom(format!(
                "kind mismatch: expected `{}`, got `{}`",
                K::KIND_STR,
                s
            )))
        }
    }
}

/// One TMS item, parameterised by kind.
///
/// `Clone`, `Debug`, `PartialEq`, `Eq` are written manually because
/// `#[derive]` would over-constrain `K` (the auto-generated bounds
/// always require `K: Clone` etc., even though `K` only appears
/// inside `PhantomData<K>` via `KindMarker<K>` and is otherwise just
/// a tag type). The status field is `Copy + Eq + Debug` via the
/// `ItemStatus` bound, so the manual impls compose without further
/// constraints.
#[derive(Serialize, Deserialize)]
#[serde(bound(
    serialize = "K::Status: Serialize",
    deserialize = "K::Status: DeserializeOwned"
))]
pub struct Item<K: ItemKind> {
    pub id: ItemId,
    #[serde(default)]
    pub kind: KindMarker<K>,
    pub claim: String,
    #[serde(default)]
    pub justifications: Vec<Justification>,
    pub status: K::Status,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<ItemId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<ItemId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defeated_by: Option<DefeatedBy>,
    pub authored_at: String,
    pub authored_in: String,
}

impl<K: ItemKind> Item<K> {
    pub fn kind_str(&self) -> &'static str {
        K::KIND_STR
    }
}

impl<K: ItemKind> Clone for Item<K> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            kind: self.kind,
            claim: self.claim.clone(),
            justifications: self.justifications.clone(),
            status: self.status,
            supersedes: self.supersedes.clone(),
            superseded_by: self.superseded_by.clone(),
            defeated_by: self.defeated_by.clone(),
            authored_at: self.authored_at.clone(),
            authored_in: self.authored_in.clone(),
        }
    }
}

impl<K: ItemKind> std::fmt::Debug for Item<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Item")
            .field("id", &self.id)
            .field("kind", &K::KIND_STR)
            .field("claim", &self.claim)
            .field("justifications", &self.justifications)
            .field("status", &self.status)
            .field("supersedes", &self.supersedes)
            .field("superseded_by", &self.superseded_by)
            .field("defeated_by", &self.defeated_by)
            .field("authored_at", &self.authored_at)
            .field("authored_in", &self.authored_in)
            .finish()
    }
}

impl<K: ItemKind> PartialEq for Item<K> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.claim == other.claim
            && self.justifications == other.justifications
            && self.status == other.status
            && self.supersedes == other.supersedes
            && self.superseded_by == other.superseded_by
            && self.defeated_by == other.defeated_by
            && self.authored_at == other.authored_at
            && self.authored_in == other.authored_in
    }
}

impl<K: ItemKind> Eq for Item<K> {}

#[cfg(test)]
pub(crate) mod test_support {
    //! Test-only kind/status types so the substrate's tests don't have
    //! to reach into a consumer's vocabulary. The shape (`Active` →
    //! `Done` | `Defeated`, with terminal `Done`/`Defeated`) is
    //! deliberately backlog-shaped so cascade tests look natural.
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum TestStatus {
        Active,
        Done,
        Defeated,
        Superseded,
    }

    impl ItemStatus for TestStatus {
        fn as_str(self) -> &'static str {
            match self {
                TestStatus::Active => "active",
                TestStatus::Done => "done",
                TestStatus::Defeated => "defeated",
                TestStatus::Superseded => "superseded",
            }
        }
        fn parse(s: &str) -> Option<Self> {
            match s {
                "active" => Some(TestStatus::Active),
                "done" => Some(TestStatus::Done),
                "defeated" => Some(TestStatus::Defeated),
                "superseded" => Some(TestStatus::Superseded),
                _ => None,
            }
        }
        fn is_terminal(self) -> bool {
            !matches!(self, TestStatus::Active)
        }
        fn transitions() -> &'static [(Self, Self)] {
            &[
                (TestStatus::Active, TestStatus::Done),
                (TestStatus::Active, TestStatus::Defeated),
                (TestStatus::Active, TestStatus::Superseded),
            ]
        }
    }

    pub struct TestKind;
    impl ItemKind for TestKind {
        type Status = TestStatus;
        const KIND_STR: &'static str = "test-item";
    }

    /// A second test kind so cross-kind cascade tests have something
    /// to talk to. Statuses are intent-shaped (`Active` →
    /// `Satisfied` | `Defeated`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum TestParentStatus {
        Active,
        Satisfied,
        Defeated,
        Superseded,
    }

    impl ItemStatus for TestParentStatus {
        fn as_str(self) -> &'static str {
            match self {
                TestParentStatus::Active => "active",
                TestParentStatus::Satisfied => "satisfied",
                TestParentStatus::Defeated => "defeated",
                TestParentStatus::Superseded => "superseded",
            }
        }
        fn parse(s: &str) -> Option<Self> {
            match s {
                "active" => Some(TestParentStatus::Active),
                "satisfied" => Some(TestParentStatus::Satisfied),
                "defeated" => Some(TestParentStatus::Defeated),
                "superseded" => Some(TestParentStatus::Superseded),
                _ => None,
            }
        }
        fn is_terminal(self) -> bool {
            !matches!(self, TestParentStatus::Active)
        }
        fn transitions() -> &'static [(Self, Self)] {
            &[
                (TestParentStatus::Active, TestParentStatus::Satisfied),
                (TestParentStatus::Active, TestParentStatus::Defeated),
                (TestParentStatus::Active, TestParentStatus::Superseded),
            ]
        }
    }

    pub struct TestParentKind;
    impl ItemKind for TestParentKind {
        type Status = TestParentStatus;
        const KIND_STR: &'static str = "test-parent";
    }

    pub fn item(id: &str, status: TestStatus, justs: Vec<Justification>) -> Item<TestKind> {
        Item {
            id: id.into(),
            kind: KindMarker::new(),
            claim: format!("claim of {id}"),
            justifications: justs,
            status,
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: "2026-04-29T00:00:00Z".into(),
            authored_in: "test".into(),
        }
    }

    pub fn parent_item(
        id: &str,
        status: TestParentStatus,
    ) -> Item<TestParentKind> {
        Item {
            id: id.into(),
            kind: KindMarker::new(),
            claim: format!("claim of {id}"),
            justifications: vec![],
            status,
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: "2026-04-29T00:00:00Z".into(),
            authored_in: "test".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn yaml_roundtrip_minimal() {
        let item = item("t-001", TestStatus::Active, vec![]);
        let yaml = serde_yaml::to_string(&item).unwrap();
        let back: Item<TestKind> = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(item, back);
    }

    #[test]
    fn yaml_emits_kind_string() {
        let item = item("t-1", TestStatus::Active, vec![]);
        let yaml = serde_yaml::to_string(&item).unwrap();
        assert!(yaml.contains("kind: test-item"), "yaml was:\n{yaml}");
    }

    #[test]
    fn yaml_rejects_kind_mismatch() {
        let yaml = "id: t-1\nkind: wrong-kind\nclaim: c\nstatus: active\nauthored_at: t\nauthored_in: test\n";
        let result: Result<Item<TestKind>, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "expected kind mismatch to fail");
    }

    #[test]
    fn defeated_by_cascade_roundtrips_as_string() {
        let yaml = "id: t-1\nkind: test-item\nclaim: c\nstatus: defeated\ndefeated_by: cascade\nauthored_at: t\nauthored_in: triage\n";
        let item: Item<TestKind> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(item.defeated_by, Some(DefeatedBy::Cascade));
        let back = serde_yaml::to_string(&item).unwrap();
        assert!(back.contains("defeated_by: cascade"));
    }

    #[test]
    fn defeated_by_item_id_roundtrips_as_string() {
        let yaml = "id: t-2\nkind: test-item\nclaim: c\nstatus: defeated\ndefeated_by: i-007\nauthored_at: t\nauthored_in: triage\n";
        let item: Item<TestKind> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(item.defeated_by, Some(DefeatedBy::Item("i-007".into())));
        let back = serde_yaml::to_string(&item).unwrap();
        assert!(back.contains("defeated_by: i-007"));
    }

    #[test]
    fn unknown_status_string_fails_to_deserialize() {
        let yaml = "id: t-1\nkind: test-item\nclaim: c\nstatus: ghost\nauthored_at: t\nauthored_in: test\n";
        let result: Result<Item<TestKind>, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err(), "expected unknown status to fail");
    }

    #[test]
    fn kind_str_is_static() {
        let item = item("t-1", TestStatus::Active, vec![]);
        assert_eq!(item.kind_str(), "test-item");
    }
}
