//! Typed schema for `<plan>/backlog.yaml`.
//!
//! Backlog items are TMS-shaped items in the `knowledge-graph`
//! substrate. The on-disk record wraps `Item<BacklogItemKind>` with
//! backlog-specific extension fields (`category`, `dependencies`,
//! `results`, `handoff`, `blocked_reason`). The `description` body
//! moves into `item.justifications` as a single `Rationale`
//! justification, mirroring the memory.yaml shape.
//!
//! See `docs/architecture-next.md` §Item shape and §Status vocabularies
//! for the rationale.

use std::collections::HashMap;

use knowledge_graph::Item;
use serde::{Deserialize, Serialize};

use crate::plan_kg::{BacklogItemKind, BacklogStatus};

pub const BACKLOG_SCHEMA_VERSION: u32 = 1;

/// One on-disk backlog entry. The flattened `item` field carries every
/// substrate-defined slot (id, kind marker, claim, justifications,
/// status, supersession links, defeat info, provenance); the remaining
/// fields are backlog-specific extensions retained from v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BacklogEntry {
    #[serde(flatten)]
    pub item: Item<BacklogItemKind>,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
}

/// The full `backlog.yaml` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklogFile {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<BacklogEntry>,
}

impl Default for BacklogFile {
    fn default() -> Self {
        Self {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: Vec::new(),
        }
    }
}

/// Per-status tally of a backlog's items. Computed via
/// `BacklogFile::task_counts` so survey (and any other caller) never
/// has to ask an LLM to count — mechanical work belongs in Rust.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCounts {
    pub total: usize,
    pub active: usize,
    pub done: usize,
    pub blocked: usize,
    pub defeated: usize,
    pub superseded: usize,
}

/// The three per-row readiness fields the survey `PlanRow` carries
/// alongside `TaskCounts`. `task_counts` is a pure per-status tally;
/// these three depend on cross-task information (dependency status) or
/// on the `handoff` field, so they live in a separate struct computed
/// in one pass. Moving them out of LLM-inferred territory lets the
/// survey prompt drop the "count these" instruction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanRowCounts {
    /// Items with `status == Active` whose every dependency id resolves
    /// to an item with `status == Done` in the same backlog. A dep id
    /// with no matching item is treated as unmet so typos or renamed
    /// ids never accidentally unblock an item.
    pub unblocked: usize,
    /// Items with `status == Blocked`, plus items with
    /// `status == Active` that have at least one unmet dep. Matches the
    /// union the survey render key documents as `B = blocked`.
    pub blocked: usize,
    /// Items carrying a `handoff` block — the YAML-era replacement for
    /// the legacy `## Received` dispatches section. Non-empty means the
    /// next triage needs to either promote the hand-off to a new item
    /// or archive it to memory.
    pub received: usize,
}

impl BacklogFile {
    /// Tally items by status. `total` is the length of the items list;
    /// the per-status fields are exact counts of items with that
    /// `BacklogStatus`. An item always contributes to exactly one
    /// per-status field, so the sum of `active + done + blocked +
    /// defeated + superseded` equals `total`.
    pub fn task_counts(&self) -> TaskCounts {
        let mut counts = TaskCounts {
            total: self.items.len(),
            ..TaskCounts::default()
        };
        for entry in &self.items {
            match entry.item.status {
                BacklogStatus::Active => counts.active += 1,
                BacklogStatus::Done => counts.done += 1,
                BacklogStatus::Blocked => counts.blocked += 1,
                BacklogStatus::Defeated => counts.defeated += 1,
                BacklogStatus::Superseded => counts.superseded += 1,
            }
        }
        counts
    }

    /// One-pass computation of the three survey-row fields whose
    /// derivation requires cross-task information. Keeping them
    /// together in `PlanRowCounts` guarantees all three come from a
    /// single consistent snapshot of the backlog.
    pub fn plan_row_counts(&self) -> PlanRowCounts {
        let done_by_id: HashMap<&str, bool> = self
            .items
            .iter()
            .map(|e| (e.item.id.as_str(), e.item.status == BacklogStatus::Done))
            .collect();
        let mut counts = PlanRowCounts::default();
        for entry in &self.items {
            if entry.handoff.is_some() {
                counts.received += 1;
            }
            match entry.item.status {
                BacklogStatus::Blocked => counts.blocked += 1,
                BacklogStatus::Active => {
                    let all_deps_done = entry
                        .dependencies
                        .iter()
                        .all(|id| done_by_id.get(id.as_str()).copied().unwrap_or(false));
                    if all_deps_done {
                        counts.unblocked += 1;
                    } else {
                        counts.blocked += 1;
                    }
                }
                BacklogStatus::Done | BacklogStatus::Defeated | BacklogStatus::Superseded => {}
            }
        }
        counts
    }
}

/// Derive a slug from an item title (claim). Lowercase, non-alphanumerics → `-`,
/// collapse repeats, trim leading/trailing `-`. Used at item creation;
/// the slug is persisted as `Item.id` and never recomputed on read.
pub fn slug_from_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut prev_dash = true;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Assign `slug_from_title(title)` with a numeric suffix to avoid
/// collisions with `existing_ids`. First attempt has no suffix; the
/// second is `-2`, third `-3`, etc.
pub fn allocate_id<'a>(title: &str, existing_ids: impl IntoIterator<Item = &'a str>) -> String {
    let base = slug_from_title(title);
    let existing: std::collections::HashSet<&str> = existing_ids.into_iter().collect();
    if !existing.contains(base.as_str()) {
        return base;
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use knowledge_graph::{Item, Justification, KindMarker};

    fn entry(id: &str, claim: &str, status: BacklogStatus, deps: &[&str]) -> BacklogEntry {
        BacklogEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: claim.into(),
                justifications: vec![Justification::Rationale {
                    text: "Body.\n".into(),
                }],
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            category: "maintenance".into(),
            blocked_reason: if status == BacklogStatus::Blocked {
                Some("upstream".into())
            } else {
                None
            },
            dependencies: deps.iter().map(|s| (*s).into()).collect(),
            results: if status == BacklogStatus::Done {
                Some("Done.\n".into())
            } else {
                None
            },
            handoff: None,
        }
    }

    #[test]
    fn entry_round_trips_through_yaml() {
        let e = entry("ex", "Example claim", BacklogStatus::Active, &[]);
        let yaml = serde_yaml::to_string(&e).unwrap();
        let decoded: BacklogEntry = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, e);
    }

    #[test]
    fn entry_yaml_carries_kind_string() {
        let e = entry("ex", "Example", BacklogStatus::Active, &[]);
        let yaml = serde_yaml::to_string(&e).unwrap();
        assert!(yaml.contains("kind: backlog-item"), "yaml was:\n{yaml}");
    }

    #[test]
    fn backlog_file_default_has_current_schema_version() {
        let file = BacklogFile::default();
        assert_eq!(file.schema_version, BACKLOG_SCHEMA_VERSION);
        assert!(file.items.is_empty());
    }

    #[test]
    fn backlog_file_round_trips_through_yaml() {
        let file = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![entry("a", "Claim A", BacklogStatus::Active, &[])],
        };
        let yaml = serde_yaml::to_string(&file).unwrap();
        let decoded: BacklogFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.schema_version, BACKLOG_SCHEMA_VERSION);
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items[0].item.id, "a");
        assert_eq!(decoded.items[0].item.claim, "Claim A");
    }

    #[test]
    fn slug_from_title_lowercases_and_punctuation_maps_to_dash() {
        assert_eq!(
            slug_from_title("Add clippy `-D warnings` CI gate"),
            "add-clippy-d-warnings-ci-gate"
        );
        assert_eq!(
            slug_from_title("Research: expose plan-state data"),
            "research-expose-plan-state-data"
        );
        assert_eq!(
            slug_from_title("  trim leading/trailing  "),
            "trim-leading-trailing"
        );
    }

    #[test]
    fn allocate_id_suffixes_on_collision() {
        let existing = ["foo", "foo-2"];
        assert_eq!(allocate_id("Foo", existing), "foo-3");
        assert_eq!(allocate_id("Foo!", existing), "foo-3");
        assert_eq!(allocate_id("Bar", existing), "bar");
    }

    #[test]
    fn task_counts_tallies_every_status_and_sums_to_total() {
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                entry("a", "A", BacklogStatus::Active, &[]),
                entry("b", "B", BacklogStatus::Active, &[]),
                entry("c", "C", BacklogStatus::Done, &[]),
                entry("d", "D", BacklogStatus::Blocked, &[]),
                entry("e", "E", BacklogStatus::Defeated, &[]),
                entry("f", "F", BacklogStatus::Superseded, &[]),
            ],
        };
        let counts = backlog.task_counts();
        assert_eq!(counts.total, 6);
        assert_eq!(counts.active, 2);
        assert_eq!(counts.done, 1);
        assert_eq!(counts.blocked, 1);
        assert_eq!(counts.defeated, 1);
        assert_eq!(counts.superseded, 1);
        assert_eq!(
            counts.active + counts.done + counts.blocked + counts.defeated + counts.superseded,
            counts.total,
            "per-status sum must equal total"
        );
    }

    #[test]
    fn task_counts_on_empty_backlog_is_all_zero() {
        let backlog = BacklogFile::default();
        let counts = backlog.task_counts();
        assert_eq!(counts, TaskCounts::default());
        assert_eq!(counts.total, 0);
    }

    #[test]
    fn plan_row_counts_unblocked_requires_every_dep_done() {
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                entry("dep-done", "Dep done", BacklogStatus::Done, &[]),
                entry("dep-active", "Dep active", BacklogStatus::Active, &[]),
                entry("ready", "Ready", BacklogStatus::Active, &["dep-done"]),
                entry("waiting", "Waiting", BacklogStatus::Active, &["dep-active"]),
                entry(
                    "partially-ready",
                    "Partially ready",
                    BacklogStatus::Active,
                    &["dep-done", "dep-active"],
                ),
            ],
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 2, "dep-active itself unblocked + ready");
        assert_eq!(
            counts.blocked, 2,
            "waiting + partially-ready are blocked on unmet deps"
        );
    }

    #[test]
    fn plan_row_counts_active_with_no_deps_is_unblocked() {
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                entry("a", "A", BacklogStatus::Active, &[]),
                entry("b", "B", BacklogStatus::Active, &[]),
            ],
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 2);
        assert_eq!(counts.blocked, 0);
    }

    #[test]
    fn plan_row_counts_unknown_dep_id_counts_as_unmet() {
        // A dep id that no item in the backlog matches is treated as
        // unmet — a typo or renamed id must not accidentally unblock
        // the item.
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![entry(
                "orphan",
                "Orphan",
                BacklogStatus::Active,
                &["nonexistent-id"],
            )],
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 0);
        assert_eq!(counts.blocked, 1);
    }

    #[test]
    fn plan_row_counts_status_blocked_always_counts_as_blocked() {
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                entry("explicitly-blocked", "X", BacklogStatus::Blocked, &[]),
                entry("blocked-with-done-deps", "Y", BacklogStatus::Blocked, &["foo"]),
                entry("foo", "Foo", BacklogStatus::Done, &[]),
            ],
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 0);
        assert_eq!(counts.blocked, 2);
    }

    #[test]
    fn plan_row_counts_terminal_states_neither_unblocked_nor_blocked() {
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                entry("d", "D", BacklogStatus::Done, &[]),
                entry("x", "X", BacklogStatus::Defeated, &[]),
                entry("s", "S", BacklogStatus::Superseded, &[]),
            ],
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 0);
        assert_eq!(counts.blocked, 0);
    }

    #[test]
    fn plan_row_counts_received_counts_items_with_handoff_regardless_of_status() {
        let mut with_handoff = entry("with-handoff", "W", BacklogStatus::Done, &[]);
        with_handoff.handoff = Some("pending design\n".into());
        let mut pending_handoff = entry("pending-handoff", "P", BacklogStatus::Active, &[]);
        pending_handoff.handoff = Some("received dispatch\n".into());
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                entry("no-handoff", "N", BacklogStatus::Active, &[]),
                with_handoff,
                pending_handoff,
            ],
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.received, 2);
        assert_eq!(
            counts.unblocked, 2,
            "no-handoff and pending-handoff both ready"
        );
    }

    #[test]
    fn plan_row_counts_empty_backlog_is_all_zero() {
        let backlog = BacklogFile::default();
        let counts = backlog.plan_row_counts();
        assert_eq!(counts, PlanRowCounts::default());
    }

    #[test]
    fn backlog_file_rejects_yaml_without_schema_version() {
        let yaml = "items: []\n";
        let result: Result<BacklogFile, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "schema_version is required; missing must fail"
        );
    }
}
