//! Handlers for every `findings <verb>` CLI verb.
//!
//! Findings are TMS items in the knowledge-graph substrate. `add`
//! lifts a `(claim, body)` pair into TMS form by mapping the body to a
//! single `Justification::Rationale`; `set-status` validates against
//! the typed `FindingStatus::transitions()` table. Mirrors
//! `state intents` in shape; the file lives at `<context>/findings.yaml`
//! rather than `<plan>/intents.yaml` because findings are context-level.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};

use knowledge_graph::{Item, ItemStatus, Justification, KindMarker};

use crate::plan_kg::FindingStatus;
use crate::state::backlog::schema::allocate_id;

use super::schema::{FindingEntry, FindingsFile, FINDINGS_SCHEMA_VERSION};
use super::yaml_io::{read_findings, write_findings};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OutputFormat {
    Yaml,
    Json,
}

impl OutputFormat {
    pub fn parse(input: &str) -> Option<OutputFormat> {
        match input {
            "yaml" => Some(OutputFormat::Yaml),
            "json" => Some(OutputFormat::Json),
            _ => None,
        }
    }
}

pub fn run_list(context_root: &Path, format: OutputFormat) -> Result<()> {
    let findings = read_findings(context_root)?;
    emit(&findings, format)
}

pub fn run_show(context_root: &Path, id: &str, format: OutputFormat) -> Result<()> {
    let findings = read_findings(context_root)?;
    let entry = find_entry(&findings, id)?;
    let wrapper = FindingsFile {
        schema_version: FINDINGS_SCHEMA_VERSION,
        items: vec![entry.clone()],
    };
    emit(&wrapper, format)
}

pub(crate) fn find_entry<'a>(findings: &'a FindingsFile, id: &str) -> Result<&'a FindingEntry> {
    findings
        .items
        .iter()
        .find(|e| e.item.id == id)
        .ok_or_else(|| anyhow::anyhow!("no finding with id {id:?}"))
}

fn emit(findings: &FindingsFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(findings)?,
        OutputFormat::Json => serde_json::to_string_pretty(findings)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub claim: String,
    pub body: String,
    pub component: Option<String>,
    pub raised_in: Option<String>,
    pub authored_at: Option<String>,
    pub authored_in: Option<String>,
}

pub fn run_add(context_root: &Path, req: &AddRequest) -> Result<()> {
    let mut findings = read_findings(context_root)?;
    let id = allocate_id(&req.claim, findings.items.iter().map(|e| e.item.id.as_str()));
    let entry = FindingEntry {
        item: Item {
            id,
            kind: KindMarker::new(),
            claim: req.claim.clone(),
            justifications: vec![Justification::Rationale {
                text: ensure_trailing_newline(&req.body),
            }],
            status: FindingStatus::New,
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: req.authored_at.clone().unwrap_or_else(current_utc_rfc3339),
            authored_in: req.authored_in.clone().unwrap_or_else(|| "unspecified".into()),
        },
        component: req.component.clone(),
        raised_in: req.raised_in.clone(),
    };
    findings.items.push(entry);
    write_findings(context_root, &findings)
}

pub fn run_set_status(context_root: &Path, id: &str, status_str: &str) -> Result<()> {
    let new_status = FindingStatus::parse(status_str).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown finding status {status_str:?}; expected one of `new`, `promoted`, `wontfix`, `superseded`"
        )
    })?;
    let mut findings = read_findings(context_root)?;
    let entry = findings
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| anyhow::anyhow!("no finding with id {id:?}"))?;
    let from = entry.item.status;
    if from == new_status {
        return Ok(());
    }
    let legal = FindingStatus::transitions()
        .iter()
        .any(|(f, t)| *f == from && *t == new_status);
    if !legal {
        bail!(
            "illegal status transition for finding {id:?}: `{}` → `{}` is not in the transition table",
            from.as_str(),
            new_status.as_str()
        );
    }
    entry.item.status = new_status;
    write_findings(context_root, &findings)
}

fn ensure_trailing_newline(body: &str) -> String {
    if body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}\n")
    }
}

/// Render the current UTC time as RFC-3339 (`YYYY-MM-DDTHH:MM:SSZ`).
/// Mirrors `state::intents::verbs::current_utc_rfc3339`. Promote to a
/// shared helper if a fourth call site appears (today there are three:
/// memory, intents, and findings).
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

    fn sample_entry(id: &str, claim: &str, rationale: &str) -> FindingEntry {
        FindingEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: claim.into(),
                justifications: vec![Justification::Rationale {
                    text: rationale.into(),
                }],
                status: FindingStatus::New,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-30T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            component: None,
            raised_in: None,
        }
    }

    fn sample_findings() -> FindingsFile {
        FindingsFile {
            schema_version: FINDINGS_SCHEMA_VERSION,
            items: vec![
                sample_entry("foo", "Foo finding", "Foo body.\n"),
                sample_entry("bar", "Bar finding", "Bar body.\n"),
            ],
        }
    }

    fn add_request(claim: &str, body: &str) -> AddRequest {
        AddRequest {
            claim: claim.into(),
            body: body.into(),
            component: None,
            raised_in: None,
            authored_at: Some("2026-04-30T00:00:00Z".into()),
            authored_in: Some("test".into()),
        }
    }

    #[test]
    fn find_entry_returns_entry_by_id() {
        let findings = sample_findings();
        let entry = find_entry(&findings, "bar").unwrap();
        assert_eq!(entry.item.id, "bar");
    }

    #[test]
    fn find_entry_errors_when_id_not_found() {
        let findings = sample_findings();
        let err = find_entry(&findings, "nonexistent").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must include bad id: {msg}");
    }

    #[test]
    fn run_add_appends_entry_with_allocated_id() {
        let tmp = TempDir::new().unwrap();
        write_findings(tmp.path(), &sample_findings()).unwrap();

        run_add(
            tmp.path(),
            &add_request("New Finding", "Body of new finding.\n"),
        )
        .unwrap();

        let updated = read_findings(tmp.path()).unwrap();
        assert_eq!(updated.items.last().unwrap().item.id, "new-finding");
        assert_eq!(updated.items.last().unwrap().item.claim, "New Finding");
        assert_eq!(updated.items.last().unwrap().item.status, FindingStatus::New);
    }

    #[test]
    fn run_add_works_when_findings_yaml_does_not_yet_exist() {
        // The inbox starts empty; first add must lazily create the file.
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), &add_request("First", "First body.\n")).unwrap();

        let updated = read_findings(tmp.path()).unwrap();
        assert_eq!(updated.items.len(), 1);
        assert_eq!(updated.items[0].item.id, "first");
    }

    #[test]
    fn run_add_records_component_and_raised_in() {
        let tmp = TempDir::new().unwrap();
        let mut req = add_request("With Attribution", "Body.\n");
        req.component = Some("atlas:atlas-ontology".into());
        req.raised_in = Some("plan/foo".into());
        run_add(tmp.path(), &req).unwrap();

        let updated = read_findings(tmp.path()).unwrap();
        let entry = updated.items.last().unwrap();
        assert_eq!(entry.component.as_deref(), Some("atlas:atlas-ontology"));
        assert_eq!(entry.raised_in.as_deref(), Some("plan/foo"));
    }

    #[test]
    fn run_add_records_rationale_as_first_justification() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), &add_request("Title", "Rationale text.\n")).unwrap();

        let updated = read_findings(tmp.path()).unwrap();
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
        write_findings(tmp.path(), &sample_findings()).unwrap();

        run_set_status(tmp.path(), "foo", "promoted").unwrap();

        let updated = read_findings(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        assert_eq!(foo.item.status, FindingStatus::Promoted);
    }

    #[test]
    fn run_set_status_rejects_unknown_status_string() {
        let tmp = TempDir::new().unwrap();
        write_findings(tmp.path(), &sample_findings()).unwrap();

        let err = run_set_status(tmp.path(), "foo", "ghost").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ghost"), "error must cite the unknown value: {msg}");
        assert!(msg.contains("new"), "error must list legal values: {msg}");
        assert!(msg.contains("wontfix"), "error must list legal values: {msg}");
    }

    #[test]
    fn run_set_status_rejects_illegal_transition() {
        let tmp = TempDir::new().unwrap();
        let mut findings = sample_findings();
        findings.items[0].item.status = FindingStatus::Promoted;
        write_findings(tmp.path(), &findings).unwrap();

        // Promoted is terminal; can't go back to New.
        let err = run_set_status(tmp.path(), "foo", "new").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("illegal"), "error must say illegal: {msg}");
        assert!(msg.contains("promoted"), "error must show from-state: {msg}");
        assert!(msg.contains("new"), "error must show to-state: {msg}");
    }

    #[test]
    fn run_set_status_self_transition_is_a_noop() {
        let tmp = TempDir::new().unwrap();
        write_findings(tmp.path(), &sample_findings()).unwrap();

        run_set_status(tmp.path(), "foo", "new").unwrap();

        let updated = read_findings(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        assert_eq!(foo.item.status, FindingStatus::New);
    }

    #[test]
    fn run_set_status_errors_on_unknown_id() {
        let tmp = TempDir::new().unwrap();
        write_findings(tmp.path(), &sample_findings()).unwrap();

        let err = run_set_status(tmp.path(), "nonexistent", "promoted").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must cite the bad id: {msg}");
    }
}
