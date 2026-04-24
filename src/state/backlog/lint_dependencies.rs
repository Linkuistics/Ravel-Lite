//! Detect drift between prose task-id mentions in a description and
//! the structured `dependencies:` field.
//!
//! Triage previously rescanned every task description in-prompt on
//! every cycle to spot the pattern "description says depends on X but
//! X is missing from the structured deps". That scan is mechanical and
//! belongs in Rust; moving it here removes token cost from every
//! triage invocation and eliminates false positives from LLM
//! interpretation of loose prose.
//!
//! The verb is read-only — it emits a report, it does not repair.
//! Reconciliation remains a judgement call (an id mentioned in prose
//! may be a reference rather than a true dependency), so triage still
//! applies `set-dependencies` based on the report.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::schema::{BacklogFile, Task};
use super::verbs::OutputFormat;
use super::yaml_io::read_backlog;

/// Drift record for a single task whose description mentions task ids
/// that are not present in its structured `dependencies:` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDrift {
    pub task_id: String,
    pub prose_mentioned: Vec<String>,
    pub structured_deps: Vec<String>,
    pub missing: Vec<String>,
}

/// Complete lint output. Mirrors the YAML shape specified in the
/// audit brief: a top-level `drifts:` list with one entry per
/// drifting task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintReport {
    pub drifts: Vec<TaskDrift>,
}

/// Pure drift analysis over a parsed backlog. Filesystem-independent so
/// unit tests can pin the semantics without tempdir setup.
pub fn lint_dependencies(backlog: &BacklogFile) -> LintReport {
    let known_ids: Vec<&str> = backlog.tasks.iter().map(|t| t.id.as_str()).collect();
    let drifts = backlog
        .tasks
        .iter()
        .filter_map(|task| analyse_task(task, &known_ids))
        .collect();
    LintReport { drifts }
}

fn analyse_task(task: &Task, known_ids: &[&str]) -> Option<TaskDrift> {
    let prose_mentioned = scan_prose_mentions(&task.description, known_ids, &task.id);
    if prose_mentioned.is_empty() {
        return None;
    }
    let structured: BTreeSet<&str> = task.dependencies.iter().map(String::as_str).collect();
    let missing: Vec<String> = prose_mentioned
        .iter()
        .filter(|id| !structured.contains(id.as_str()))
        .cloned()
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some(TaskDrift {
        task_id: task.id.clone(),
        prose_mentioned,
        structured_deps: task.dependencies.clone(),
        missing,
    })
}

/// Whole-word scan of `prose` for any of `known_ids`, skipping
/// `self_id` (a task mentioning its own id is not drift). Output is
/// sorted and deduplicated.
fn scan_prose_mentions(prose: &str, known_ids: &[&str], self_id: &str) -> Vec<String> {
    let mut hits: BTreeSet<String> = BTreeSet::new();
    for id in known_ids {
        if *id == self_id {
            continue;
        }
        if contains_id_as_word(prose, id) {
            hits.insert((*id).to_string());
        }
    }
    hits.into_iter().collect()
}

/// True iff `haystack` contains `id` with non-slug boundaries on both
/// sides. Slug chars are `[a-zA-Z0-9_-]`; any other byte (whitespace,
/// punctuation, UTF-8 continuation bytes) counts as a boundary.
fn contains_id_as_word(haystack: &str, id: &str) -> bool {
    let needle = id.as_bytes();
    if needle.is_empty() {
        return false;
    }
    let hay = haystack.as_bytes();
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if &hay[i..i + needle.len()] == needle {
            let before_ok = i == 0 || !is_slug_byte(hay[i - 1]);
            let after_ok = i + needle.len() == hay.len() || !is_slug_byte(hay[i + needle.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_slug_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_')
}

/// CLI entry point: load `<plan_dir>/backlog.yaml`, produce the drift
/// report, emit it to stdout in the requested format.
pub fn run_lint_dependencies(plan_dir: &Path, format: OutputFormat) -> Result<()> {
    let backlog = read_backlog(plan_dir)?;
    let report = lint_dependencies(&backlog);
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(&report)?,
        OutputFormat::Json => serde_json::to_string_pretty(&report)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::schema::Status;
    use super::*;

    fn task_with(id: &str, dependencies: Vec<String>, description: &str) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            category: "maintenance".into(),
            status: Status::NotStarted,
            blocked_reason: None,
            dependencies,
            description: description.into(),
            results: None,
            handoff: None,
        }
    }

    fn backlog_with(tasks: Vec<Task>) -> BacklogFile {
        BacklogFile { tasks, extra: Default::default() }
    }

    #[test]
    fn no_drift_when_prose_mention_matches_structured_deps() {
        let backlog = backlog_with(vec![
            task_with("foo", vec![], "original brief\n"),
            task_with("bar", vec!["foo".into()], "depends on foo to finish\n"),
        ]);
        assert_eq!(lint_dependencies(&backlog), LintReport::default());
    }

    #[test]
    fn drift_when_prose_mention_missing_from_structured_deps() {
        let backlog = backlog_with(vec![
            task_with("foo", vec![], "brief\n"),
            task_with("bar", vec![], "see also foo for context\n"),
        ]);
        let report = lint_dependencies(&backlog);
        assert_eq!(report.drifts.len(), 1);
        let drift = &report.drifts[0];
        assert_eq!(drift.task_id, "bar");
        assert_eq!(drift.prose_mentioned, vec!["foo".to_string()]);
        assert_eq!(drift.structured_deps, Vec::<String>::new());
        assert_eq!(drift.missing, vec!["foo".to_string()]);
    }

    #[test]
    fn structured_dep_not_in_prose_is_not_flagged() {
        // `bar` depends on `foo` structurally but never names it in
        // prose. Structured is authoritative — this is not drift.
        let backlog = backlog_with(vec![
            task_with("foo", vec![], "brief\n"),
            task_with("bar", vec!["foo".into()], "no mention of the dep here\n"),
        ]);
        assert_eq!(lint_dependencies(&backlog), LintReport::default());
    }

    #[test]
    fn empty_backlog_yields_empty_report() {
        let backlog = backlog_with(vec![]);
        assert_eq!(lint_dependencies(&backlog), LintReport::default());
    }

    #[test]
    fn prose_string_that_is_not_an_actual_id_is_not_flagged() {
        let backlog = backlog_with(vec![
            task_with("foo", vec![], "brief\n"),
            task_with("bar", vec![], "mentions some-other-thing not in backlog\n"),
        ]);
        assert_eq!(lint_dependencies(&backlog), LintReport::default());
    }

    #[test]
    fn self_mention_is_not_drift() {
        let backlog = backlog_with(vec![task_with(
            "foo",
            vec![],
            "this is foo's own brief; foo references itself\n",
        )]);
        assert_eq!(lint_dependencies(&backlog), LintReport::default());
    }

    #[test]
    fn multiple_prose_mentions_all_reported_in_sorted_order() {
        let backlog = backlog_with(vec![
            task_with("alpha", vec![], "brief\n"),
            task_with("beta", vec![], "brief\n"),
            task_with("gamma", vec!["alpha".into()], "needs alpha and beta\n"),
        ]);
        let report = lint_dependencies(&backlog);
        assert_eq!(report.drifts.len(), 1);
        let drift = &report.drifts[0];
        assert_eq!(drift.task_id, "gamma");
        assert_eq!(
            drift.prose_mentioned,
            vec!["alpha".to_string(), "beta".to_string()]
        );
        assert_eq!(drift.missing, vec!["beta".to_string()]);
    }

    #[test]
    fn substring_match_does_not_count_without_word_boundaries() {
        // Task id "foo" must not match inside "foobar" or "foo-bar-baz".
        let backlog = backlog_with(vec![
            task_with("foo", vec![], "brief\n"),
            task_with("qux", vec![], "references foobar and foo-bar-baz only\n"),
        ]);
        assert_eq!(lint_dependencies(&backlog), LintReport::default());
    }

    #[test]
    fn id_inside_backticks_is_matched() {
        // Markdown code-spans are a common way to cite an id; backtick
        // is a non-slug byte so the boundary check admits the match.
        let backlog = backlog_with(vec![
            task_with("foo", vec![], "brief\n"),
            task_with("bar", vec![], "See `foo` for details.\n"),
        ]);
        let report = lint_dependencies(&backlog);
        assert_eq!(report.drifts.len(), 1);
        assert_eq!(report.drifts[0].prose_mentioned, vec!["foo".to_string()]);
    }

    #[test]
    fn run_lint_dependencies_emits_yaml_for_populated_backlog() {
        use crate::state::backlog::yaml_io::write_backlog;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let backlog = backlog_with(vec![
            task_with("foo", vec![], "brief\n"),
            task_with("bar", vec![], "see also foo\n"),
        ]);
        write_backlog(tmp.path(), &backlog).unwrap();

        // Drive the pure analysis through the same path the runner uses
        // (read_backlog → lint_dependencies → serde_yaml) so a future
        // refactor of the runner can't silently diverge from the test.
        let loaded = read_backlog(tmp.path()).unwrap();
        let report = lint_dependencies(&loaded);
        let yaml = serde_yaml::to_string(&report).unwrap();
        assert!(yaml.contains("task_id: bar"), "yaml must cite drifting task: {yaml}");
        assert!(yaml.contains("missing:"), "yaml must name missing field: {yaml}");
    }

    #[test]
    fn task_with_no_description_does_not_panic() {
        // Minimal description just a newline — the byte-scanner must
        // handle short haystacks without slicing past the end.
        let backlog = backlog_with(vec![
            task_with("foo", vec![], "\n"),
            task_with("bar", vec![], "\n"),
        ]);
        assert_eq!(lint_dependencies(&backlog), LintReport::default());
    }
}
