//! Computes a human-readable summary of what changed in a plan's
//! `backlog.yaml` between a baseline commit and the current working-tree
//! state. Injected into the analyse-work prompt as `{{BACKLOG_TRANSITIONS}}`
//! so the LLM can author a specific commit title ("Mark <task-id> done,
//! record results") rather than falling back to the phase name.
//!
//! Runner-side on purpose: scanning a baseline YAML and a current YAML to
//! compute a delta is pure mechanical work — exactly the kind of thing the
//! "Never do in an LLM what you can do in code" rule says should live here.
//!
//! Soft-fails on every failure mode (missing baseline, git error, parse
//! error) because the prompt's `{{BACKLOG_TRANSITIONS}}` slot always needs
//! *something*; an `Err` would bubble up into `compose_prompt` and wedge
//! the phase loop — exactly the same rationale as `work_tree_snapshot`.

use std::path::Path;
use std::process::Command;

use knowledge_graph::ItemStatus;

use crate::plan_kg::BacklogStatus;
use crate::state::backlog::schema::{BacklogEntry, BacklogFile};
use crate::state::filenames::BACKLOG_FILENAME;

/// Top-level entry point used by `phase_loop`. Always returns a printable
/// string; never propagates an error.
pub fn backlog_transitions(plan_dir: &Path, baseline_sha: &str) -> String {
    if baseline_sha.is_empty() {
        return "(no baseline SHA available; first cycle has no prior state to diff)".to_string();
    }

    let current = match crate::state::backlog::read_backlog(plan_dir) {
        Ok(b) => b,
        Err(e) => return format!("(failed to read current {BACKLOG_FILENAME}: {e})"),
    };

    let baseline = match read_baseline_backlog(plan_dir, baseline_sha) {
        BaselineResult::Ok(b) => b,
        BaselineResult::Missing => {
            return render_additions_only(&current);
        }
        BaselineResult::Error(msg) => return format!("(baseline lookup failed: {msg})"),
    };

    let transitions = compute_transitions(&baseline, &current);
    render_transitions(&transitions)
}

enum BaselineResult {
    Ok(BacklogFile),
    Missing,
    Error(String),
}

/// Retrieve `backlog.yaml` content at `baseline_sha`. Uses
/// `git ls-files --full-name <backlog>` to resolve the path
/// relative to the git repo root, so this works identically in a
/// single-repo layout and in a monorepo subtree.
fn read_baseline_backlog(plan_dir: &Path, baseline_sha: &str) -> BaselineResult {
    let full_name_out = Command::new("git")
        .current_dir(plan_dir)
        .args(["ls-files", "--full-name", BACKLOG_FILENAME])
        .output();

    let full_name = match full_name_out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Ok(o) => {
            return BaselineResult::Error(format!(
                "git ls-files exited {}: {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
        Err(e) => return BaselineResult::Error(format!("git ls-files failed: {e}")),
    };

    if full_name.is_empty() {
        return BaselineResult::Missing;
    }

    let show_out = Command::new("git")
        .current_dir(plan_dir)
        .args(["show", &format!("{baseline_sha}:{full_name}")])
        .output();

    match show_out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).into_owned();
            match serde_yaml::from_str::<BacklogFile>(&text) {
                Ok(b) => BaselineResult::Ok(b),
                Err(e) => BaselineResult::Error(format!("baseline YAML parse: {e}")),
            }
        }
        Ok(_) => BaselineResult::Missing,
        Err(e) => BaselineResult::Error(format!("git show failed: {e}")),
    }
}

/// Transition record for one item id. Absent-in-baseline and
/// absent-in-current are the two "pure" cases; everything else is a
/// field-level diff where at least one of status/results/title/deps
/// differs.
#[derive(Debug, Default, PartialEq, Eq)]
struct Transitions {
    status_flips: Vec<(String, BacklogStatus, BacklogStatus)>,
    results_added: Vec<(String, usize)>,
    results_modified: Vec<(String, usize, usize)>,
    tasks_added: Vec<(String, String)>,
    tasks_deleted: Vec<(String, String)>,
    title_changes: Vec<(String, String, String)>,
    dependency_changes: Vec<(String, Vec<String>, Vec<String>)>,
    handoff_changes: Vec<(String, HandoffChange)>,
}

#[derive(Debug, PartialEq, Eq)]
enum HandoffChange {
    Added,
    Modified,
    Cleared,
}

fn compute_transitions(baseline: &BacklogFile, current: &BacklogFile) -> Transitions {
    use std::collections::HashMap;

    let base_by_id: HashMap<&str, &BacklogEntry> = baseline
        .items
        .iter()
        .map(|e| (e.item.id.as_str(), e))
        .collect();
    let curr_by_id: HashMap<&str, &BacklogEntry> = current
        .items
        .iter()
        .map(|e| (e.item.id.as_str(), e))
        .collect();

    let mut out = Transitions::default();

    for curr in &current.items {
        match base_by_id.get(curr.item.id.as_str()) {
            None => {
                out.tasks_added
                    .push((curr.item.id.clone(), curr.item.claim.clone()));
            }
            Some(base) => {
                diff_entry_fields(base, curr, &mut out);
            }
        }
    }

    for base in &baseline.items {
        if !curr_by_id.contains_key(base.item.id.as_str()) {
            out.tasks_deleted
                .push((base.item.id.clone(), base.item.claim.clone()));
        }
    }

    out
}

fn diff_entry_fields(base: &BacklogEntry, curr: &BacklogEntry, out: &mut Transitions) {
    if base.item.status != curr.item.status {
        out.status_flips
            .push((curr.item.id.clone(), base.item.status, curr.item.status));
    }

    let base_results_len = base.results.as_deref().map(line_count).unwrap_or(0);
    let curr_results_len = curr.results.as_deref().map(line_count).unwrap_or(0);
    match (base_results_len, curr_results_len) {
        (0, n) if n > 0 => out.results_added.push((curr.item.id.clone(), n)),
        (b, c) if b > 0 && c > 0 && base.results != curr.results => {
            out.results_modified.push((curr.item.id.clone(), b, c));
        }
        _ => {}
    }

    if base.item.claim != curr.item.claim {
        out.title_changes.push((
            curr.item.id.clone(),
            base.item.claim.clone(),
            curr.item.claim.clone(),
        ));
    }

    if base.dependencies != curr.dependencies {
        out.dependency_changes.push((
            curr.item.id.clone(),
            base.dependencies.clone(),
            curr.dependencies.clone(),
        ));
    }

    let change = match (base.handoff.as_deref(), curr.handoff.as_deref()) {
        (None, Some(s)) if !s.is_empty() => Some(HandoffChange::Added),
        (Some(b), None) if !b.is_empty() => Some(HandoffChange::Cleared),
        (Some(b), Some(c)) if b != c => Some(HandoffChange::Modified),
        _ => None,
    };
    if let Some(hc) = change {
        out.handoff_changes.push((curr.item.id.clone(), hc));
    }
}

fn line_count(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count()
    }
}

fn render_transitions(t: &Transitions) -> String {
    let mut sections: Vec<String> = Vec::new();

    if !t.status_flips.is_empty() {
        let mut lines = vec!["status flips:".to_string()];
        for (id, from, to) in &t.status_flips {
            lines.push(format!("  - {id}: {} → {}", from.as_str(), to.as_str()));
        }
        sections.push(lines.join("\n"));
    }

    if !t.results_added.is_empty() {
        let mut lines = vec!["results added:".to_string()];
        for (id, n) in &t.results_added {
            lines.push(format!("  - {id}: {n} line(s)"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.results_modified.is_empty() {
        let mut lines = vec!["results modified:".to_string()];
        for (id, from, to) in &t.results_modified {
            lines.push(format!("  - {id}: {from} → {to} line(s)"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.tasks_added.is_empty() {
        let mut lines = vec!["tasks added:".to_string()];
        for (id, title) in &t.tasks_added {
            lines.push(format!("  + {id}: {title}"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.tasks_deleted.is_empty() {
        let mut lines = vec!["tasks deleted:".to_string()];
        for (id, title) in &t.tasks_deleted {
            lines.push(format!("  - {id}: {title}"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.title_changes.is_empty() {
        let mut lines = vec!["title changes:".to_string()];
        for (id, from, to) in &t.title_changes {
            lines.push(format!("  - {id}: {from:?} → {to:?}"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.dependency_changes.is_empty() {
        let mut lines = vec!["dependency changes:".to_string()];
        for (id, from, to) in &t.dependency_changes {
            lines.push(format!("  - {id}: {from:?} → {to:?}"));
        }
        sections.push(lines.join("\n"));
    }

    if !t.handoff_changes.is_empty() {
        let mut lines = vec!["handoff changes:".to_string()];
        for (id, change) in &t.handoff_changes {
            let label = match change {
                HandoffChange::Added => "added",
                HandoffChange::Modified => "modified",
                HandoffChange::Cleared => "cleared",
            };
            lines.push(format!("  - {id}: {label}"));
        }
        sections.push(lines.join("\n"));
    }

    if sections.is_empty() {
        "(no backlog changes since baseline)".to_string()
    } else {
        sections.join("\n\n")
    }
}

/// Fallback renderer for the "baseline-backlog-missing" case, which
/// happens on a plan's very first cycle: every item in the current
/// backlog is "new" by definition, so we render additions only.
fn render_additions_only(current: &BacklogFile) -> String {
    if current.items.is_empty() {
        return "(no baseline; current backlog is empty)".to_string();
    }
    let mut lines = vec![
        "(no baseline found at this SHA — rendering current backlog as additions)".to_string(),
        String::new(),
        "tasks added:".to_string(),
    ];
    for entry in &current.items {
        lines.push(format!("  + {}: {}", entry.item.id, entry.item.claim));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::backlog::schema::{BacklogEntry, BACKLOG_SCHEMA_VERSION};
    use knowledge_graph::{Item, Justification, KindMarker};

    fn entry(id: &str, status: BacklogStatus) -> BacklogEntry {
        BacklogEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: format!("Task {id}"),
                justifications: vec![Justification::Rationale {
                    text: "desc".into(),
                }],
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "test".into(),
                authored_in: "test".into(),
            },
            category: "core".into(),
            blocked_reason: None,
            dependencies: vec![],
            results: None,
            handoff: None,
        }
    }

    fn file(items: Vec<BacklogEntry>) -> BacklogFile {
        BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items,
        }
    }

    #[test]
    fn empty_baseline_and_current_renders_no_change_marker() {
        let base = BacklogFile::default();
        let curr = BacklogFile::default();
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert_eq!(rendered, "(no backlog changes since baseline)");
    }

    #[test]
    fn status_flip_is_rendered_with_arrow() {
        let base = file(vec![entry("foo", BacklogStatus::Active)]);
        let curr = file(vec![entry("foo", BacklogStatus::Done)]);
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert!(rendered.contains("status flips:"), "missing header: {rendered}");
        assert!(rendered.contains("foo: active → done"), "missing line: {rendered}");
    }

    #[test]
    fn results_added_counts_lines() {
        let base = file(vec![entry("foo", BacklogStatus::Done)]);
        let mut with_results = entry("foo", BacklogStatus::Done);
        with_results.results = Some("line one\nline two\nline three".into());
        let curr = file(vec![with_results]);
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert!(rendered.contains("results added:"), "missing header: {rendered}");
        assert!(rendered.contains("foo: 3 line(s)"), "missing line: {rendered}");
    }

    #[test]
    fn results_modified_shows_old_and_new_line_count() {
        let mut base_entry = entry("foo", BacklogStatus::Done);
        base_entry.results = Some("old\ntext".into());
        let mut curr_entry = entry("foo", BacklogStatus::Done);
        curr_entry.results = Some("new\ntext\nmore\nlines".into());

        let t = compute_transitions(&file(vec![base_entry]), &file(vec![curr_entry]));
        let rendered = render_transitions(&t);
        assert!(rendered.contains("results modified:"), "missing header: {rendered}");
        assert!(rendered.contains("foo: 2 → 4 line(s)"), "missing line: {rendered}");
    }

    #[test]
    fn added_task_renders_with_plus_marker() {
        let base = BacklogFile::default();
        let curr = file(vec![entry("new-id", BacklogStatus::Active)]);
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert!(rendered.contains("tasks added:"), "missing header: {rendered}");
        assert!(rendered.contains("+ new-id: Task new-id"), "missing line: {rendered}");
    }

    #[test]
    fn deleted_task_renders_with_minus_marker() {
        let base = file(vec![entry("old-id", BacklogStatus::Done)]);
        let curr = BacklogFile::default();
        let t = compute_transitions(&base, &curr);
        let rendered = render_transitions(&t);
        assert!(rendered.contains("tasks deleted:"), "missing header: {rendered}");
        assert!(rendered.contains("- old-id: Task old-id"), "missing line: {rendered}");
    }

    #[test]
    fn dependency_changes_are_reported() {
        let mut base_entry = entry("foo", BacklogStatus::Active);
        base_entry.dependencies = vec!["dep-a".into()];
        let mut curr_entry = entry("foo", BacklogStatus::Active);
        curr_entry.dependencies = vec!["dep-a".into(), "dep-b".into()];
        let t = compute_transitions(&file(vec![base_entry]), &file(vec![curr_entry]));
        let rendered = render_transitions(&t);
        assert!(rendered.contains("dependency changes:"), "missing header: {rendered}");
        assert!(rendered.contains("foo:"), "missing id line: {rendered}");
    }

    #[test]
    fn handoff_added_is_classified_correctly() {
        let base_entry = entry("foo", BacklogStatus::Done);
        let mut curr_entry = entry("foo", BacklogStatus::Done);
        curr_entry.handoff = Some("design decision".into());
        let t = compute_transitions(&file(vec![base_entry]), &file(vec![curr_entry]));
        assert_eq!(t.handoff_changes, vec![("foo".into(), HandoffChange::Added)]);
    }

    #[test]
    fn empty_baseline_sha_yields_explanatory_placeholder() {
        let tmp = tempfile::TempDir::new().unwrap();
        let out = backlog_transitions(tmp.path(), "");
        assert!(out.contains("no baseline SHA"), "expected placeholder, got: {out}");
    }

    /// End-to-end: baseline backlog committed in git, current backlog
    /// on disk differs, `backlog_transitions` reports the diff.
    #[test]
    fn backlog_transitions_reads_baseline_from_git_show() {
        use std::process::Command;

        let tmp = tempfile::TempDir::new().unwrap();
        let plan = tmp.path();
        Command::new("git").current_dir(plan).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["config", "user.name", "t"]).output().unwrap();

        let base = file(vec![entry("foo", BacklogStatus::Active)]);
        crate::state::backlog::write_backlog(plan, &base).unwrap();
        Command::new("git").current_dir(plan).args(["add", "-A"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["commit", "-q", "-m", "baseline"]).output().unwrap();
        let sha = String::from_utf8(
            Command::new("git").current_dir(plan).args(["rev-parse", "HEAD"]).output().unwrap().stdout,
        ).unwrap().trim().to_string();

        let curr = file(vec![entry("foo", BacklogStatus::Done)]);
        crate::state::backlog::write_backlog(plan, &curr).unwrap();

        let rendered = backlog_transitions(plan, &sha);
        assert!(rendered.contains("foo: active → done"), "expected status flip, got: {rendered}");
    }

    /// First-cycle case: baseline commit predates backlog.yaml.
    #[test]
    fn backlog_transitions_handles_missing_baseline_file() {
        use std::process::Command;

        let tmp = tempfile::TempDir::new().unwrap();
        let plan = tmp.path();
        Command::new("git").current_dir(plan).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["config", "user.name", "t"]).output().unwrap();

        std::fs::write(plan.join("README"), "seed\n").unwrap();
        Command::new("git").current_dir(plan).args(["add", "-A"]).output().unwrap();
        Command::new("git").current_dir(plan).args(["commit", "-q", "-m", "seed"]).output().unwrap();
        let sha = String::from_utf8(
            Command::new("git").current_dir(plan).args(["rev-parse", "HEAD"]).output().unwrap().stdout,
        ).unwrap().trim().to_string();

        let curr = file(vec![entry("foo", BacklogStatus::Active)]);
        crate::state::backlog::write_backlog(plan, &curr).unwrap();

        let rendered = backlog_transitions(plan, &sha);
        assert!(rendered.contains("no baseline found"), "expected first-cycle marker: {rendered}");
        assert!(rendered.contains("+ foo: Task foo"), "expected additions-only block: {rendered}");
    }
}
