//! Typed schema for `<plan>/backlog.yaml`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    NotStarted,
    InProgress,
    Done,
    Blocked,
}

impl Status {
    pub fn parse(input: &str) -> Option<Status> {
        match input {
            "not_started" => Some(Status::NotStarted),
            "in_progress" => Some(Status::InProgress),
            "done" => Some(Status::Done),
            "blocked" => Some(Status::Blocked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub category: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BacklogFile {
    #[serde(default)]
    pub tasks: Vec<Task>,
    /// Preserve unknown top-level keys so a roundtrip through older readers
    /// never drops fields a newer writer added. Future-proofs the schema
    /// against the R2–R5 extensions.
    #[serde(flatten)]
    pub extra: IndexMap<String, serde_yaml::Value>,
}

/// Per-status tally of a backlog's tasks. Computed from a parsed
/// `BacklogFile` via `BacklogFile::task_counts` so survey (and any
/// other caller) never has to ask an LLM to count — mechanical work
/// belongs in Rust.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCounts {
    pub total: usize,
    pub not_started: usize,
    pub in_progress: usize,
    pub done: usize,
    pub blocked: usize,
}

/// The three per-row readiness fields the survey `PlanRow` carries
/// alongside `TaskCounts`. `task_counts` is a pure per-status tally;
/// these three depend on cross-task information (dependency status) or
/// on the `handoff` field, so they live in a separate struct computed
/// in one pass. Moving them out of LLM-inferred territory lets the
/// survey prompt drop the "count these" instruction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanRowCounts {
    /// Tasks with `status == NotStarted` whose every dependency id
    /// resolves to a task with `status == Done` in the same backlog.
    /// A dep id with no matching task is treated as unmet so typos or
    /// renamed ids never accidentally unblock a task.
    pub unblocked: usize,
    /// Tasks with `status == Blocked`, plus tasks with
    /// `status == NotStarted` that have at least one unmet dep. Matches
    /// the union the survey render key at `src/survey/render.rs`
    /// documents as `B = blocked`.
    pub blocked: usize,
    /// Tasks carrying a `handoff` block — the YAML-era replacement for
    /// the legacy `## Received` dispatches section. Non-empty means the
    /// next triage needs to either promote the hand-off to a new task
    /// or archive it to memory.
    pub received: usize,
}

impl BacklogFile {
    /// Tally tasks by status. `total` is the length of the task list;
    /// the per-status fields are exact counts of tasks with that
    /// `Status`. A task always contributes to exactly one per-status
    /// field, so the sum of `not_started + in_progress + done + blocked`
    /// equals `total`.
    pub fn task_counts(&self) -> TaskCounts {
        let mut counts = TaskCounts {
            total: self.tasks.len(),
            ..TaskCounts::default()
        };
        for task in &self.tasks {
            match task.status {
                Status::NotStarted => counts.not_started += 1,
                Status::InProgress => counts.in_progress += 1,
                Status::Done => counts.done += 1,
                Status::Blocked => counts.blocked += 1,
            }
        }
        counts
    }

    /// One-pass computation of the three survey-row fields whose
    /// derivation requires cross-task information. Keeping them together
    /// in `PlanRowCounts` guarantees all three come from a single
    /// consistent snapshot of the backlog.
    pub fn plan_row_counts(&self) -> PlanRowCounts {
        use std::collections::HashMap;
        let done_by_id: HashMap<&str, bool> = self
            .tasks
            .iter()
            .map(|t| (t.id.as_str(), t.status == Status::Done))
            .collect();
        let mut counts = PlanRowCounts::default();
        for task in &self.tasks {
            if task.handoff.is_some() {
                counts.received += 1;
            }
            match task.status {
                Status::Blocked => counts.blocked += 1,
                Status::NotStarted => {
                    let all_deps_done = task
                        .dependencies
                        .iter()
                        .all(|id| done_by_id.get(id.as_str()).copied().unwrap_or(false));
                    if all_deps_done {
                        counts.unblocked += 1;
                    } else {
                        counts.blocked += 1;
                    }
                }
                Status::InProgress | Status::Done => {}
            }
        }
        counts
    }
}

/// Derive a slug from a task title. Lowercase, non-alphanumerics → `-`,
/// collapse repeats, trim leading/trailing `-`. Used at task creation;
/// the slug is persisted as `Task::id` and never recomputed on read.
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

    #[test]
    fn status_round_trips_through_yaml() {
        for status in [Status::NotStarted, Status::InProgress, Status::Done, Status::Blocked] {
            let serialised = serde_yaml::to_string(&status).unwrap();
            let parsed: Status = serde_yaml::from_str(&serialised).unwrap();
            assert_eq!(status, parsed, "roundtrip failed for {status:?}");
        }
    }

    #[test]
    fn status_parse_accepts_snake_case_cli_input() {
        assert_eq!(Status::parse("not_started"), Some(Status::NotStarted));
        assert_eq!(Status::parse("in_progress"), Some(Status::InProgress));
        assert_eq!(Status::parse("done"), Some(Status::Done));
        assert_eq!(Status::parse("blocked"), Some(Status::Blocked));
        assert_eq!(Status::parse("NotStarted"), None);
        assert_eq!(Status::parse(""), None);
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
    fn task_round_trips_preserving_optional_fields() {
        let task = Task {
            id: "example".into(),
            title: "Example task".into(),
            category: "maintenance".into(),
            status: Status::NotStarted,
            blocked_reason: None,
            dependencies: vec![],
            description: "Body.\n".into(),
            results: None,
            handoff: None,
        };
        let yaml = serde_yaml::to_string(&task).unwrap();
        // `skip_serializing_if` keeps optional nulls out of the wire form.
        assert!(!yaml.contains("blocked_reason"), "optional None must not emit: {yaml}");
        assert!(!yaml.contains("results"), "optional None must not emit: {yaml}");
        assert!(!yaml.contains("handoff"), "optional None must not emit: {yaml}");
        let decoded: Task = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded.id, task.id);
        assert_eq!(decoded.status, task.status);
    }

    #[test]
    fn task_counts_tallies_every_status_and_sums_to_total() {
        fn task(status: Status) -> Task {
            Task {
                id: format!("t-{status:?}").to_lowercase(),
                title: "t".into(),
                category: "maintenance".into(),
                status,
                blocked_reason: if status == Status::Blocked {
                    Some("upstream".into())
                } else {
                    None
                },
                dependencies: vec![],
                description: "body\n".into(),
                results: None,
                handoff: None,
            }
        }
        let backlog = BacklogFile {
            tasks: vec![
                task(Status::NotStarted),
                task(Status::NotStarted),
                task(Status::InProgress),
                task(Status::Done),
                task(Status::Blocked),
            ],
            extra: Default::default(),
        };
        let counts = backlog.task_counts();
        assert_eq!(counts.total, 5);
        assert_eq!(counts.not_started, 2);
        assert_eq!(counts.in_progress, 1);
        assert_eq!(counts.done, 1);
        assert_eq!(counts.blocked, 1);
        assert_eq!(
            counts.not_started + counts.in_progress + counts.done + counts.blocked,
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

    fn t(id: &str, status: Status, deps: &[&str]) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            category: "maintenance".into(),
            status,
            blocked_reason: if status == Status::Blocked {
                Some("upstream".into())
            } else {
                None
            },
            dependencies: deps.iter().map(|s| (*s).into()).collect(),
            description: "Body.\n".into(),
            results: if status == Status::Done {
                Some("Done.\n".into())
            } else {
                None
            },
            handoff: None,
        }
    }

    #[test]
    fn plan_row_counts_unblocked_requires_every_dep_done() {
        let backlog = BacklogFile {
            tasks: vec![
                t("dep-done", Status::Done, &[]),
                t("dep-in-progress", Status::InProgress, &[]),
                t("ready", Status::NotStarted, &["dep-done"]),
                t("waiting", Status::NotStarted, &["dep-in-progress"]),
                t("partially-ready", Status::NotStarted, &["dep-done", "dep-in-progress"]),
            ],
            extra: Default::default(),
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 1);
        assert_eq!(counts.blocked, 2, "waiting + partially-ready are blocked on unmet deps");
    }

    #[test]
    fn plan_row_counts_not_started_with_no_deps_is_unblocked() {
        let backlog = BacklogFile {
            tasks: vec![
                t("a", Status::NotStarted, &[]),
                t("b", Status::NotStarted, &[]),
            ],
            extra: Default::default(),
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 2);
        assert_eq!(counts.blocked, 0);
    }

    #[test]
    fn plan_row_counts_unknown_dep_id_counts_as_unmet() {
        // A dep id that no task in the backlog matches is treated as
        // unmet — a typo or renamed id must not accidentally unblock
        // the task.
        let backlog = BacklogFile {
            tasks: vec![t("orphan", Status::NotStarted, &["nonexistent-id"])],
            extra: Default::default(),
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 0);
        assert_eq!(counts.blocked, 1);
    }

    #[test]
    fn plan_row_counts_status_blocked_always_counts_as_blocked() {
        let backlog = BacklogFile {
            tasks: vec![
                t("explicitly-blocked", Status::Blocked, &[]),
                t("blocked-with-done-deps", Status::Blocked, &["foo"]),
                t("foo", Status::Done, &[]),
            ],
            extra: Default::default(),
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 0);
        assert_eq!(counts.blocked, 2);
    }

    #[test]
    fn plan_row_counts_in_progress_and_done_are_neither_unblocked_nor_blocked() {
        let backlog = BacklogFile {
            tasks: vec![
                t("in-flight", Status::InProgress, &[]),
                t("finished", Status::Done, &[]),
            ],
            extra: Default::default(),
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.unblocked, 0);
        assert_eq!(counts.blocked, 0);
    }

    #[test]
    fn plan_row_counts_received_counts_tasks_with_handoff_regardless_of_status() {
        let mut with_handoff = t("with-handoff", Status::Done, &[]);
        with_handoff.handoff = Some("pending design\n".into());
        let mut pending_handoff = t("pending-handoff", Status::NotStarted, &[]);
        pending_handoff.handoff = Some("received dispatch\n".into());
        let backlog = BacklogFile {
            tasks: vec![
                t("no-handoff", Status::NotStarted, &[]),
                with_handoff,
                pending_handoff,
            ],
            extra: Default::default(),
        };
        let counts = backlog.plan_row_counts();
        assert_eq!(counts.received, 2);
        // Unrelated fields still compute correctly around handoffs.
        assert_eq!(counts.unblocked, 2, "no-handoff and pending-handoff both ready");
    }

    #[test]
    fn plan_row_counts_empty_backlog_is_all_zero() {
        let backlog = BacklogFile::default();
        let counts = backlog.plan_row_counts();
        assert_eq!(counts, PlanRowCounts::default());
    }

    #[test]
    fn backlog_file_preserves_unknown_top_level_keys() {
        // Future schema extensions (R2+) may add top-level keys. The flatten
        // extra buffer keeps them alive across an R1 read/write cycle.
        let input = r#"
tasks: []
schema_version: 1
"#;
        let parsed: BacklogFile = serde_yaml::from_str(input).unwrap();
        assert!(parsed.extra.contains_key("schema_version"));
        let re_emitted = serde_yaml::to_string(&parsed).unwrap();
        assert!(re_emitted.contains("schema_version"), "extra keys must round-trip: {re_emitted}");
    }
}
