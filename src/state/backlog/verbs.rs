//! Handlers for every `state backlog <verb>` CLI verb.
//!
//! Each handler is a thin wrapper around the schema + yaml_io: pull the
//! file, mutate or project it, write it back (for mutations) or emit to
//! stdout (for reads). Filter predicates for `list` live here too; they
//! are pure functions against the BacklogFile so unit tests can pin
//! semantics without touching the filesystem.
//!
//! `run_set_status` is wired through `Store<BacklogItemKind>::set_status`
//! so transition validation is enforced at the CLI boundary against the
//! typed `BacklogStatus::transitions()` table — illegal moves like
//! `done → active` fail loudly rather than corrupting the file.

use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use knowledge_graph::{Item, Justification, KindMarker, Store};

use crate::bail_with;
use crate::cli::{CodedError, ErrorCode, OutputFormat};
use crate::plan_kg::{BacklogItemKind, BacklogStatus};

use super::render::{render_markdown, GroupBy};
use super::schema::{
    allocate_id, BacklogEntry, BacklogFile, BACKLOG_SCHEMA_VERSION,
};
use super::yaml_io::{read_backlog, write_backlog};

/// "no item with id X in backlog" — the recurring NotFound error in
/// every per-id verb. Lifted to a helper because the inline form is
/// four lines repeated nine times.
fn not_found_in_backlog(id: &str) -> anyhow::Error {
    anyhow::Error::new(CodedError {
        code: ErrorCode::NotFound,
        message: format!("no item with id {id:?} in backlog"),
    })
}

#[derive(Debug, Default, Clone)]
pub struct ListFilter {
    pub status: Option<BacklogStatus>,
    pub category: Option<String>,
    pub ready: bool,
    pub has_handoff: bool,
    pub missing_results: bool,
}

/// True iff `entry` passes every filter constraint set in `filter`.
pub fn entry_matches(
    entry: &BacklogEntry,
    filter: &ListFilter,
    done_ids: &HashSet<&str>,
) -> bool {
    if let Some(status) = filter.status {
        if entry.item.status != status {
            return false;
        }
    }
    if let Some(category) = &filter.category {
        if &entry.category != category {
            return false;
        }
    }
    if filter.ready {
        // `--ready` = `status == active AND every dep is done`.
        if entry.item.status != BacklogStatus::Active {
            return false;
        }
        if entry
            .dependencies
            .iter()
            .any(|dep| !done_ids.contains(dep.as_str()))
        {
            return false;
        }
    }
    if filter.has_handoff && entry.handoff.is_none() {
        return false;
    }
    if filter.missing_results
        && !(entry.item.status == BacklogStatus::Done && entry.results.is_none())
    {
        return false;
    }
    true
}

pub fn run_list(
    plan_dir: &Path,
    filter: &ListFilter,
    format: OutputFormat,
    group_by: GroupBy,
) -> Result<()> {
    let backlog = read_backlog(plan_dir)?;
    let done_ids: HashSet<&str> = backlog
        .items
        .iter()
        .filter(|e| e.item.status == BacklogStatus::Done)
        .map(|e| e.item.id.as_str())
        .collect();

    let filtered: Vec<&BacklogEntry> = backlog
        .items
        .iter()
        .filter(|e| entry_matches(e, filter, &done_ids))
        .collect();

    let projection = BacklogFile {
        schema_version: BACKLOG_SCHEMA_VERSION,
        items: filtered.into_iter().cloned().collect(),
    };

    match format {
        OutputFormat::Markdown => {
            print!("{}", render_markdown(&projection, group_by));
            Ok(())
        }
        _ => emit(&projection, format),
    }
}

fn emit(backlog: &BacklogFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(backlog)?,
        OutputFormat::Json => serde_json::to_string_pretty(backlog)? + "\n",
        OutputFormat::Markdown => {
            bail_with!(
                ErrorCode::InvalidInput,
                "markdown output is only supported on `backlog list`; got it from a different verb"
            )
        }
    };
    print!("{serialised}");
    Ok(())
}

pub fn run_show(plan_dir: &Path, id: &str, format: OutputFormat) -> Result<()> {
    if format == OutputFormat::Markdown {
        bail_with!(
            ErrorCode::InvalidInput,
            "`backlog show` does not support --format markdown; use yaml or json"
        );
    }
    let backlog = read_backlog(plan_dir)?;
    let entry = find_entry(&backlog, id)?;
    let wrapper = BacklogFile {
        schema_version: BACKLOG_SCHEMA_VERSION,
        items: vec![entry.clone()],
    };
    emit(&wrapper, format)
}

pub(crate) fn find_entry<'a>(backlog: &'a BacklogFile, id: &str) -> Result<&'a BacklogEntry> {
    backlog
        .items
        .iter()
        .find(|e| e.item.id == id)
        .ok_or_else(|| not_found_in_backlog(id))
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub title: String,
    pub category: String,
    pub dependencies: Vec<String>,
    pub description: String,
}

pub fn run_add(plan_dir: &Path, req: &AddRequest) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;

    // Validate deps up-front so a typo surfaces as an error rather than
    // a dangling reference in the stored file.
    let existing_ids: HashSet<&str> =
        backlog.items.iter().map(|e| e.item.id.as_str()).collect();
    for dep in &req.dependencies {
        if !existing_ids.contains(dep.as_str()) {
            bail_with!(
                ErrorCode::NotFound,
                "dependency id {dep:?} does not exist in backlog; known ids: {:?}",
                existing_ids
            );
        }
    }

    let id = allocate_id(&req.title, backlog.items.iter().map(|e| e.item.id.as_str()));
    backlog.items.push(BacklogEntry {
        item: Item {
            id,
            kind: KindMarker::new(),
            claim: req.title.clone(),
            justifications: vec![Justification::Rationale {
                text: ensure_trailing_newline(&req.description),
            }],
            status: BacklogStatus::Active,
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: current_utc_rfc3339(),
            authored_in: "unspecified".into(),
        },
        category: req.category.clone(),
        blocked_reason: None,
        dependencies: req.dependencies.clone(),
        results: None,
        handoff: None,
    });
    write_backlog(plan_dir, &backlog)
}

pub fn run_init(plan_dir: &Path, seed: &BacklogFile) -> Result<()> {
    let existing = read_backlog(plan_dir)?;
    if !existing.items.is_empty() {
        bail_with!(
            ErrorCode::Conflict,
            "refusing to init: backlog.yaml at {} is non-empty ({} items). Use `add` for incremental inserts.",
            plan_dir.display(),
            existing.items.len()
        );
    }
    write_backlog(plan_dir, seed)
}

fn ensure_trailing_newline(body: &str) -> String {
    if body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}\n")
    }
}

/// Apply a status mutation through the substrate's typed validation.
/// Builds a `Store<BacklogItemKind>` from the current items, calls
/// `Store::set_status` (which checks the typed transition table), then
/// copies the mutated `Item` back into the backlog entry. Extension
/// fields on `BacklogEntry` (category, dependencies, results, handoff,
/// blocked_reason) stay attached to the entry — they don't go through
/// the Store.
pub fn run_set_status(
    plan_dir: &Path,
    id: &str,
    status: BacklogStatus,
    reason: Option<&str>,
) -> Result<()> {
    if status == BacklogStatus::Blocked && reason.is_none() {
        bail_with!(
            ErrorCode::InvalidInput,
            "--reason <text> is required when setting status to `blocked`"
        );
    }
    let mut backlog = read_backlog(plan_dir)?;
    if !backlog.items.iter().any(|e| e.item.id == id) {
        return Err(not_found_in_backlog(id));
    }

    let mut store: Store<BacklogItemKind> = Store::new();
    for entry in &backlog.items {
        store.insert(entry.item.clone())?;
    }
    store.set_status(id, status)?;
    let mutated_item = store
        .get(id)
        .expect("just set; id confirmed present above")
        .clone();

    let entry = backlog
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .expect("id confirmed present above");
    entry.item = mutated_item;
    entry.blocked_reason = if status == BacklogStatus::Blocked {
        reason.map(str::to_string)
    } else {
        None
    };
    write_backlog(plan_dir, &backlog)
}

pub fn run_set_results(plan_dir: &Path, id: &str, body: &str) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;
    let entry = backlog
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| not_found_in_backlog(id))?;
    entry.results = Some(ensure_trailing_newline(body));
    write_backlog(plan_dir, &backlog)
}

/// Rewrite an item's description (the rationale justification's body).
/// Body is required to be non-empty — the field has a non-empty
/// invariant enforced at `add` time and a blind-replace that violated
/// it would poison the item brief.
pub fn run_set_description(plan_dir: &Path, id: &str, body: &str) -> Result<()> {
    if body.trim().is_empty() {
        bail_with!(
            ErrorCode::InvalidInput,
            "description body must not be empty or whitespace-only"
        );
    }
    let mut backlog = read_backlog(plan_dir)?;
    let entry = backlog
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| not_found_in_backlog(id))?;
    let new_text = ensure_trailing_newline(body);
    let rationale_slot = entry
        .item
        .justifications
        .iter_mut()
        .find(|j| matches!(j, Justification::Rationale { .. }));
    if let Some(j) = rationale_slot {
        *j = Justification::Rationale { text: new_text };
    } else {
        entry
            .item
            .justifications
            .insert(0, Justification::Rationale { text: new_text });
    }
    write_backlog(plan_dir, &backlog)
}

pub fn run_set_handoff(plan_dir: &Path, id: &str, body: &str) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;
    let entry = backlog
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| not_found_in_backlog(id))?;
    entry.handoff = Some(ensure_trailing_newline(body));
    write_backlog(plan_dir, &backlog)
}

pub fn run_clear_handoff(plan_dir: &Path, id: &str) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;
    let entry = backlog
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| not_found_in_backlog(id))?;
    entry.handoff = None;
    write_backlog(plan_dir, &backlog)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ReorderPosition {
    Before,
    After,
}

impl ReorderPosition {
    pub fn parse(input: &str) -> Option<ReorderPosition> {
        match input {
            "before" => Some(ReorderPosition::Before),
            "after" => Some(ReorderPosition::After),
            _ => None,
        }
    }
}

pub fn run_set_title(plan_dir: &Path, id: &str, new_title: &str) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;
    let entry = backlog
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| not_found_in_backlog(id))?;
    entry.item.claim = new_title.to_string();
    // id is intentionally not recomputed — its stability is the whole
    // point of persisting the slug at creation.
    write_backlog(plan_dir, &backlog)
}

/// Replace the `dependencies` field on an item post-hoc. `run_add`
/// validates the same way at creation time, but because deps there must
/// already exist, `add` cannot introduce a cycle; `set-dependencies`
/// can, so the cycle check is additional here.
pub fn run_set_dependencies(plan_dir: &Path, id: &str, deps: &[String]) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;

    let target_index = backlog
        .items
        .iter()
        .position(|e| e.item.id == id)
        .ok_or_else(|| not_found_in_backlog(id))?;

    if deps.iter().any(|d| d == id) {
        bail_with!(
            ErrorCode::InvalidInput,
            "item {id:?} cannot depend on itself"
        );
    }

    let existing_ids: HashSet<&str> =
        backlog.items.iter().map(|e| e.item.id.as_str()).collect();
    for dep in deps {
        if !existing_ids.contains(dep.as_str()) {
            bail_with!(
                ErrorCode::NotFound,
                "dependency id {dep:?} does not exist in backlog; known ids: {:?}",
                existing_ids
            );
        }
    }

    // Cycle check: adding edge id → d would close a cycle iff the
    // existing graph already has a path d → … → id.
    for dep in deps {
        if dependency_path_exists(&backlog, dep, id) {
            bail_with!(
                ErrorCode::InvalidInput,
                "refusing to set dependencies on {id:?}: would create a cycle through {dep:?}"
            );
        }
    }

    backlog.items[target_index].dependencies = deps.to_vec();
    write_backlog(plan_dir, &backlog)
}

/// True iff following `dependencies` edges from `from` can reach `to`.
fn dependency_path_exists(backlog: &BacklogFile, from: &str, to: &str) -> bool {
    let mut stack = vec![from];
    let mut visited: HashSet<&str> = HashSet::new();
    while let Some(current) = stack.pop() {
        if current == to {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(entry) = backlog.items.iter().find(|e| e.item.id == current) {
            for dep in &entry.dependencies {
                stack.push(dep.as_str());
            }
        }
    }
    false
}

pub fn run_reorder(
    plan_dir: &Path,
    id: &str,
    position: ReorderPosition,
    target_id: &str,
) -> Result<()> {
    if id == target_id {
        bail_with!(
            ErrorCode::InvalidInput,
            "cannot reorder an item relative to itself"
        );
    }
    let mut backlog = read_backlog(plan_dir)?;
    let source_index = backlog
        .items
        .iter()
        .position(|e| e.item.id == id)
        .ok_or_else(|| not_found_in_backlog(id))?;
    let entry = backlog.items.remove(source_index);

    let target_index = backlog
        .items
        .iter()
        .position(|e| e.item.id == target_id)
        .ok_or_else(|| anyhow::Error::new(CodedError {
            code: ErrorCode::NotFound,
            message: format!("no target item with id {target_id:?} in backlog"),
        }))?;

    let insert_at = match position {
        ReorderPosition::Before => target_index,
        ReorderPosition::After => target_index + 1,
    };
    backlog.items.insert(insert_at, entry);
    write_backlog(plan_dir, &backlog)
}

pub fn run_delete(plan_dir: &Path, id: &str, force: bool) -> Result<()> {
    let mut backlog = read_backlog(plan_dir)?;

    let dependents: Vec<String> = backlog
        .items
        .iter()
        .filter(|e| e.dependencies.iter().any(|dep| dep == id))
        .map(|e| e.item.id.clone())
        .collect();

    if !dependents.is_empty() && !force {
        bail_with!(
            ErrorCode::Conflict,
            "refusing to delete {id}: item is a dependency of {:?}. Rerun with --force to cascade-remove the dep references.",
            dependents
        );
    }

    if force {
        for entry in backlog.items.iter_mut() {
            entry.dependencies.retain(|dep| dep != id);
        }
    }

    let before = backlog.items.len();
    backlog.items.retain(|e| e.item.id != id);
    if backlog.items.len() == before {
        return Err(not_found_in_backlog(id));
    }
    write_backlog(plan_dir, &backlog)
}

/// Render the current UTC time as RFC-3339 (`YYYY-MM-DDTHH:MM:SSZ`).
/// Matches the formatting used by `state memory verbs` so backlog and
/// memory items written in the same wall-second carry equal timestamps.
fn current_utc_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_utc(secs)
}

fn format_unix_utc(mut secs: u64) -> String {
    let seconds = (secs % 60) as u32;
    secs /= 60;
    let minutes = (secs % 60) as u32;
    secs /= 60;
    let hours = (secs % 24) as u32;
    let mut days = secs / 24;

    let mut year: u32 = 1970;
    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if days < year_days as u64 {
            break;
        }
        days -= year_days as u64;
        year += 1;
    }

    let month_lens: [u32; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month: u32 = 0;
    while month < 12 && days >= month_lens[month as usize] as u64 {
        days -= month_lens[month as usize] as u64;
        month += 1;
    }
    let day = (days as u32) + 1;
    let month_1based = month + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month_1based, day, hours, minutes, seconds
    )
}

fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::backlog::yaml_io::write_backlog;
    use tempfile::TempDir;

    fn make_entry(id: &str, status: BacklogStatus, deps: &[&str]) -> BacklogEntry {
        BacklogEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: id.into(),
                justifications: vec![Justification::Rationale {
                    text: "body\n".into(),
                }],
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "test".into(),
                authored_in: "test".into(),
            },
            category: "maintenance".into(),
            blocked_reason: None,
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            results: if status == BacklogStatus::Done {
                Some("did it\n".into())
            } else {
                None
            },
            handoff: None,
        }
    }

    fn sample_backlog() -> BacklogFile {
        BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                make_entry("foo", BacklogStatus::Done, &[]),
                make_entry("bar", BacklogStatus::Active, &["foo"]),
                make_entry("baz", BacklogStatus::Active, &["bar"]),
                make_entry("qux", BacklogStatus::Active, &[]),
            ],
        }
    }

    fn done_ids(backlog: &BacklogFile) -> HashSet<&str> {
        backlog
            .items
            .iter()
            .filter(|e| e.item.status == BacklogStatus::Done)
            .map(|e| e.item.id.as_str())
            .collect()
    }

    #[test]
    fn ready_filter_excludes_items_with_unmet_deps() {
        let backlog = sample_backlog();
        let done = done_ids(&backlog);
        let filter = ListFilter { ready: true, ..Default::default() };
        let matches: Vec<&str> = backlog
            .items
            .iter()
            .filter(|e| entry_matches(e, &filter, &done))
            .map(|e| e.item.id.as_str())
            .collect();
        // `bar` is active AND its only dep (`foo`) is done → ready.
        // `baz` is active BUT its dep (`bar`) is not done → not ready.
        // `foo` is done → excluded by status=active check.
        // `qux` is active with no deps → ready.
        assert_eq!(matches, vec!["bar", "qux"]);
    }

    #[test]
    fn status_filter_narrows_to_exact_match() {
        let backlog = sample_backlog();
        let done = done_ids(&backlog);
        let filter = ListFilter {
            status: Some(BacklogStatus::Active),
            ..Default::default()
        };
        let matches: Vec<&str> = backlog
            .items
            .iter()
            .filter(|e| entry_matches(e, &filter, &done))
            .map(|e| e.item.id.as_str())
            .collect();
        assert_eq!(matches, vec!["bar", "baz", "qux"]);
    }

    #[test]
    fn missing_results_filter_matches_done_items_without_results() {
        let mut backlog = sample_backlog();
        backlog.items[0].results = None;
        let done = done_ids(&backlog);
        let filter = ListFilter {
            missing_results: true,
            ..Default::default()
        };
        let matches: Vec<&str> = backlog
            .items
            .iter()
            .filter(|e| entry_matches(e, &filter, &done))
            .map(|e| e.item.id.as_str())
            .collect();
        assert_eq!(matches, vec!["foo"]);
    }

    #[test]
    fn run_show_rejects_markdown_format_with_clear_error() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();
        let err = run_show(tmp.path(), "foo", OutputFormat::Markdown).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("markdown"), "error must mention markdown: {msg}");
    }

    #[test]
    fn find_entry_returns_entry_by_id() {
        let backlog = sample_backlog();
        let entry = find_entry(&backlog, "bar").unwrap();
        assert_eq!(entry.item.id, "bar");
    }

    #[test]
    fn find_entry_errors_when_id_not_found() {
        let backlog = sample_backlog();
        let err = find_entry(&backlog, "nonexistent").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must include the bad id: {msg}");
    }

    #[test]
    fn run_add_appends_entry_with_allocated_id() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let req = AddRequest {
            title: "New task".into(),
            category: "maintenance".into(),
            dependencies: vec!["foo".into()],
            description: "Description body.\n".into(),
        };
        run_add(tmp.path(), &req).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        assert_eq!(updated.items.last().unwrap().item.id, "new-task");
        assert_eq!(updated.items.last().unwrap().item.claim, "New task");
        assert_eq!(updated.items.last().unwrap().dependencies, vec!["foo"]);
        assert_eq!(updated.items.last().unwrap().item.status, BacklogStatus::Active);
    }

    #[test]
    fn run_add_errors_when_dependency_ids_are_unknown() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let req = AddRequest {
            title: "New task".into(),
            category: "maintenance".into(),
            dependencies: vec!["nonexistent".into()],
            description: "body".into(),
        };
        let err = run_add(tmp.path(), &req).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must name the missing dep: {msg}");
    }

    #[test]
    fn run_init_populates_empty_backlog() {
        let tmp = TempDir::new().unwrap();
        let initial = BacklogFile::default();
        write_backlog(tmp.path(), &initial).unwrap();

        let seed = sample_backlog();
        run_init(tmp.path(), &seed).unwrap();

        let stored = read_backlog(tmp.path()).unwrap();
        assert_eq!(stored.items.len(), seed.items.len());
    }

    #[test]
    fn run_init_refuses_to_overwrite_non_empty_backlog() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_init(tmp.path(), &sample_backlog()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("non-empty"), "error must explain the refusal: {msg}");
    }

    #[test]
    fn run_set_status_updates_the_target_item() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_status(tmp.path(), "bar", BacklogStatus::Blocked, Some("upstream")).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.items.iter().find(|e| e.item.id == "bar").unwrap();
        assert_eq!(bar.item.status, BacklogStatus::Blocked);
        assert_eq!(bar.blocked_reason.as_deref(), Some("upstream"));
    }

    #[test]
    fn run_set_status_requires_reason_for_blocked() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_status(tmp.path(), "bar", BacklogStatus::Blocked, None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("reason"), "error must mention reason: {msg}");
    }

    #[test]
    fn run_set_status_clears_reason_when_moving_out_of_blocked() {
        let tmp = TempDir::new().unwrap();
        let mut backlog = sample_backlog();
        backlog.items[1].item.status = BacklogStatus::Blocked;
        backlog.items[1].blocked_reason = Some("upstream".into());
        write_backlog(tmp.path(), &backlog).unwrap();

        run_set_status(tmp.path(), "bar", BacklogStatus::Active, None).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.items.iter().find(|e| e.item.id == "bar").unwrap();
        assert_eq!(
            bar.blocked_reason, None,
            "blocked_reason must clear when status leaves blocked"
        );
    }

    #[test]
    fn run_set_status_rejects_illegal_transition() {
        let tmp = TempDir::new().unwrap();
        let mut backlog = sample_backlog();
        backlog.items[1].item.status = BacklogStatus::Done;
        write_backlog(tmp.path(), &backlog).unwrap();

        // Done is terminal — can't go back to Active.
        let err = run_set_status(tmp.path(), "bar", BacklogStatus::Active, None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("illegal") || msg.contains("transition"),
            "error must signal an illegal transition: {msg}"
        );
    }

    #[test]
    fn run_set_status_self_transition_is_a_noop() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        // active → active: legal no-op.
        run_set_status(tmp.path(), "bar", BacklogStatus::Active, None).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.items.iter().find(|e| e.item.id == "bar").unwrap();
        assert_eq!(bar.item.status, BacklogStatus::Active);
    }

    #[test]
    fn run_set_description_rewrites_rationale_and_preserves_other_fields() {
        let tmp = TempDir::new().unwrap();
        let mut seed = sample_backlog();
        seed.items[0].results = Some("keep me\n".into());
        seed.items[0].handoff = Some("keep me too\n".into());
        write_backlog(tmp.path(), &seed).unwrap();

        run_set_description(tmp.path(), "foo", "Fresh brief.\n").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        match &foo.item.justifications[0] {
            Justification::Rationale { text } => assert_eq!(text, "Fresh brief.\n"),
            other => panic!("expected Rationale justification, got {other:?}"),
        }
        assert_eq!(foo.results.as_deref(), Some("keep me\n"));
        assert_eq!(foo.handoff.as_deref(), Some("keep me too\n"));
        assert_eq!(foo.item.status, BacklogStatus::Done);
        assert_eq!(foo.item.claim, "foo");
    }

    #[test]
    fn run_set_description_rejects_empty_body() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_description(tmp.path(), "foo", "").unwrap_err();
        assert!(format!("{err:#}").contains("empty"));
    }

    #[test]
    fn run_set_results_writes_markdown_body() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_results(tmp.path(), "foo", "Body of results.\n").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        assert_eq!(foo.results.as_deref(), Some("Body of results.\n"));
    }

    #[test]
    fn run_set_handoff_writes_markdown_body() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_handoff(tmp.path(), "foo", "Promote follow-up.\n").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        assert_eq!(foo.handoff.as_deref(), Some("Promote follow-up.\n"));
    }

    #[test]
    fn run_clear_handoff_nulls_the_field() {
        let tmp = TempDir::new().unwrap();
        let mut backlog = sample_backlog();
        backlog.items[0].handoff = Some("some handoff\n".into());
        write_backlog(tmp.path(), &backlog).unwrap();

        run_clear_handoff(tmp.path(), "foo").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        assert_eq!(foo.handoff, None);
    }

    #[test]
    fn run_set_title_updates_claim_but_preserves_id() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_title(tmp.path(), "bar", "Bar's New Title").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.items.iter().find(|e| e.item.id == "bar").unwrap();
        assert_eq!(bar.item.claim, "Bar's New Title");
        assert_eq!(bar.item.id, "bar", "id must not change when claim changes");
    }

    #[test]
    fn run_reorder_before_moves_item_to_earlier_position() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_reorder(tmp.path(), "qux", ReorderPosition::Before, "foo").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let ids: Vec<&str> = updated.items.iter().map(|e| e.item.id.as_str()).collect();
        assert_eq!(ids, vec!["qux", "foo", "bar", "baz"]);
    }

    #[test]
    fn run_reorder_after_moves_item_to_later_position() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_reorder(tmp.path(), "foo", ReorderPosition::After, "baz").unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let ids: Vec<&str> = updated.items.iter().map(|e| e.item.id.as_str()).collect();
        assert_eq!(ids, vec!["bar", "baz", "foo", "qux"]);
    }

    #[test]
    fn run_set_dependencies_replaces_the_dependency_list() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_set_dependencies(tmp.path(), "bar", &["qux".into()]).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        let bar = updated.items.iter().find(|e| e.item.id == "bar").unwrap();
        assert_eq!(bar.dependencies, vec!["qux".to_string()]);
    }

    #[test]
    fn run_set_dependencies_rejects_self_reference() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_dependencies(tmp.path(), "bar", &["bar".into()]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("itself") || msg.contains("self"),
            "error must call out the self-reference: {msg}"
        );
    }

    #[test]
    fn run_set_dependencies_rejects_cycles() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_set_dependencies(tmp.path(), "foo", &["baz".into()]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("cycle"), "error must mention cycle: {msg}");
    }

    #[test]
    fn run_delete_errors_when_item_has_dependents() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        let err = run_delete(tmp.path(), "foo", false).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("bar"), "error must cite the dependent: {msg}");
    }

    #[test]
    fn run_delete_with_force_cascades_dep_reference_cleanup() {
        let tmp = TempDir::new().unwrap();
        write_backlog(tmp.path(), &sample_backlog()).unwrap();

        run_delete(tmp.path(), "foo", true).unwrap();

        let updated = read_backlog(tmp.path()).unwrap();
        assert!(!updated.items.iter().any(|e| e.item.id == "foo"));
        let bar = updated.items.iter().find(|e| e.item.id == "bar").unwrap();
        assert!(!bar.dependencies.contains(&"foo".to_string()));
    }
}
