//! Handlers for every `state intents <verb>` CLI verb.
//!
//! Intent entries are TMS items in the knowledge-graph substrate.
//! `add` lifts a `(claim, body)` pair into the TMS form by mapping
//! the body to a single `Justification::Rationale`. `set-status`
//! validates against the typed `IntentStatus::transitions()` table.
//! Mirrors `state memory` minus the `attribution` extension and the
//! `set-body`/`set-title` verbs (intents at v1 are lightweight; if
//! editing an intent's prose proves needed, add those verbs in
//! parity with memory rather than inventing a new shape).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};

use knowledge_graph::{Item, ItemStatus, Justification, KindMarker};

use crate::cli::OutputFormat;
use crate::plan_kg::IntentStatus;
use crate::state::backlog::schema::allocate_id;

use super::schema::{IntentEntry, IntentsFile, INTENTS_SCHEMA_VERSION};
use super::yaml_io::{read_intents, write_intents};

pub fn run_list(plan_dir: &Path, format: OutputFormat) -> Result<()> {
    let intents = read_intents(plan_dir)?;
    emit(&intents, format)
}

pub fn run_show(plan_dir: &Path, id: &str, format: OutputFormat) -> Result<()> {
    let intents = read_intents(plan_dir)?;
    let entry = find_entry(&intents, id)?;
    let wrapper = IntentsFile {
        schema_version: INTENTS_SCHEMA_VERSION,
        items: vec![entry.clone()],
    };
    emit(&wrapper, format)
}

pub(crate) fn find_entry<'a>(intents: &'a IntentsFile, id: &str) -> Result<&'a IntentEntry> {
    intents
        .items
        .iter()
        .find(|e| e.item.id == id)
        .ok_or_else(|| anyhow::anyhow!("no intent with id {id:?}"))
}

fn emit(intents: &IntentsFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(intents)?,
        OutputFormat::Json => serde_json::to_string_pretty(intents)? + "\n",
        OutputFormat::Markdown => {
            bail!("`state intents` does not support --format markdown; use yaml or json")
        }
    };
    print!("{serialised}");
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub claim: String,
    pub body: String,
    pub authored_at: Option<String>,
    pub authored_in: Option<String>,
}

pub fn run_add(plan_dir: &Path, req: &AddRequest) -> Result<()> {
    let mut intents = read_intents(plan_dir)?;
    let id = allocate_id(&req.claim, intents.items.iter().map(|e| e.item.id.as_str()));
    let entry = IntentEntry {
        item: Item {
            id,
            kind: KindMarker::new(),
            claim: req.claim.clone(),
            justifications: vec![Justification::Rationale {
                text: ensure_trailing_newline(&req.body),
            }],
            status: IntentStatus::Active,
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: req.authored_at.clone().unwrap_or_else(current_utc_rfc3339),
            authored_in: req.authored_in.clone().unwrap_or_else(|| "unspecified".into()),
        },
    };
    intents.items.push(entry);
    write_intents(plan_dir, &intents)
}

pub fn run_set_status(plan_dir: &Path, id: &str, status_str: &str) -> Result<()> {
    let new_status = IntentStatus::parse(status_str).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown intent status {status_str:?}; expected one of `active`, `satisfied`, `defeated`, `superseded`"
        )
    })?;
    let mut intents = read_intents(plan_dir)?;
    let entry = intents
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| anyhow::anyhow!("no intent with id {id:?}"))?;
    let from = entry.item.status;
    if from == new_status {
        return Ok(());
    }
    let legal = IntentStatus::transitions()
        .iter()
        .any(|(f, t)| *f == from && *t == new_status);
    if !legal {
        bail!(
            "illegal status transition for intent {id:?}: `{}` → `{}` is not in the transition table",
            from.as_str(),
            new_status.as_str()
        );
    }
    entry.item.status = new_status;
    write_intents(plan_dir, &intents)
}

fn ensure_trailing_newline(body: &str) -> String {
    if body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}\n")
    }
}

/// Render the current UTC time as RFC-3339 (`YYYY-MM-DDTHH:MM:SSZ`).
/// Mirrors `state::memory::verbs::current_utc_rfc3339` rather than
/// pulling a chrono dependency. Promote to a shared module if a
/// fourth call site appears.
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
    use tempfile::TempDir;

    fn sample_entry(id: &str, claim: &str, rationale: &str) -> IntentEntry {
        IntentEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: claim.into(),
                justifications: vec![Justification::Rationale {
                    text: rationale.into(),
                }],
                status: IntentStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
        }
    }

    fn sample_intents() -> IntentsFile {
        IntentsFile {
            schema_version: INTENTS_SCHEMA_VERSION,
            items: vec![
                sample_entry("foo", "Foo claim", "Foo body.\n"),
                sample_entry("bar", "Bar claim", "Bar body.\n"),
            ],
        }
    }

    fn add_request(claim: &str, body: &str) -> AddRequest {
        AddRequest {
            claim: claim.into(),
            body: body.into(),
            authored_at: Some("2026-04-29T00:00:00Z".into()),
            authored_in: Some("test".into()),
        }
    }

    #[test]
    fn find_entry_returns_entry_by_id() {
        let intents = sample_intents();
        let entry = find_entry(&intents, "bar").unwrap();
        assert_eq!(entry.item.id, "bar");
    }

    #[test]
    fn find_entry_errors_when_id_not_found() {
        let intents = sample_intents();
        let err = find_entry(&intents, "nonexistent").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must include bad id: {msg}");
    }

    #[test]
    fn run_add_appends_entry_with_allocated_id() {
        let tmp = TempDir::new().unwrap();
        write_intents(tmp.path(), &sample_intents()).unwrap();

        run_add(tmp.path(), &add_request("New Intent", "Body of new intent.\n")).unwrap();

        let updated = read_intents(tmp.path()).unwrap();
        assert_eq!(updated.items.last().unwrap().item.id, "new-intent");
        assert_eq!(updated.items.last().unwrap().item.claim, "New Intent");
        assert_eq!(updated.items.last().unwrap().item.status, IntentStatus::Active);
    }

    #[test]
    fn run_add_records_rationale_as_first_justification() {
        let tmp = TempDir::new().unwrap();
        write_intents(tmp.path(), &IntentsFile::default()).unwrap();

        run_add(tmp.path(), &add_request("Title", "Rationale text.\n")).unwrap();

        let updated = read_intents(tmp.path()).unwrap();
        let entry = updated.items.last().unwrap();
        assert_eq!(entry.item.justifications.len(), 1);
        match &entry.item.justifications[0] {
            Justification::Rationale { text } => assert_eq!(text, "Rationale text.\n"),
            other => panic!("expected Rationale justification, got {other:?}"),
        }
    }

    #[test]
    fn run_set_status_accepts_legal_transition() {
        let tmp = TempDir::new().unwrap();
        write_intents(tmp.path(), &sample_intents()).unwrap();

        run_set_status(tmp.path(), "foo", "satisfied").unwrap();

        let updated = read_intents(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        assert_eq!(foo.item.status, IntentStatus::Satisfied);
    }

    #[test]
    fn run_set_status_rejects_unknown_status_string() {
        let tmp = TempDir::new().unwrap();
        write_intents(tmp.path(), &sample_intents()).unwrap();

        let err = run_set_status(tmp.path(), "foo", "ghost").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ghost"), "error must cite the unknown value: {msg}");
        assert!(msg.contains("active"), "error must list legal values: {msg}");
        assert!(msg.contains("satisfied"), "error must list legal values: {msg}");
    }

    #[test]
    fn run_set_status_rejects_illegal_transition() {
        let tmp = TempDir::new().unwrap();
        let mut intents = sample_intents();
        intents.items[0].item.status = IntentStatus::Satisfied;
        write_intents(tmp.path(), &intents).unwrap();

        // Satisfied is terminal; can't go back to Active.
        let err = run_set_status(tmp.path(), "foo", "active").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("illegal"), "error must say illegal: {msg}");
        assert!(msg.contains("satisfied"), "error must show from-state: {msg}");
        assert!(msg.contains("active"), "error must show to-state: {msg}");
    }

    #[test]
    fn run_set_status_self_transition_is_a_noop() {
        let tmp = TempDir::new().unwrap();
        write_intents(tmp.path(), &sample_intents()).unwrap();

        // active → active: legal no-op.
        run_set_status(tmp.path(), "foo", "active").unwrap();

        let updated = read_intents(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        assert_eq!(foo.item.status, IntentStatus::Active);
    }

    #[test]
    fn run_set_status_errors_on_unknown_id() {
        let tmp = TempDir::new().unwrap();
        write_intents(tmp.path(), &sample_intents()).unwrap();

        let err = run_set_status(tmp.path(), "nonexistent", "satisfied").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must cite the bad id: {msg}");
    }
}
