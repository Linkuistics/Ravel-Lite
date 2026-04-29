//! Handlers for every `state memory <verb>` CLI verb.
//!
//! Memory entries are TMS items in the knowledge-graph substrate.
//! `add` lifts the legacy `(title, body)` shape into the TMS form by
//! mapping `title` → `claim` and `body` → a single `Rationale`
//! justification; `set-body` and `set-title` preserve the old verb
//! names for phase-prompt continuity by acting on the rationale's
//! text and the claim respectively. `set-status` is new and routes
//! through the typed `MemoryStatus::transitions()` table for
//! validated transitions; the substrate's `Store` API is
//! deliberately bypassed here because attribution preservation
//! across an extract-mutate-rebuild cycle would dwarf the savings.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};

use knowledge_graph::{Item, ItemStatus, Justification, KindMarker};

use crate::plan_kg::MemoryStatus;
use crate::state::backlog::schema::allocate_id;

use super::schema::{MemoryEntry, MemoryFile, MEMORY_SCHEMA_VERSION};
use super::yaml_io::{read_memory, write_memory};

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

pub fn run_list(plan_dir: &Path, format: OutputFormat) -> Result<()> {
    let memory = read_memory(plan_dir)?;
    emit(&memory, format)
}

pub fn run_show(plan_dir: &Path, id: &str, format: OutputFormat) -> Result<()> {
    let memory = read_memory(plan_dir)?;
    let entry = find_entry(&memory, id)?;
    let wrapper = MemoryFile {
        schema_version: MEMORY_SCHEMA_VERSION,
        items: vec![entry.clone()],
    };
    emit(&wrapper, format)
}

pub(crate) fn find_entry<'a>(memory: &'a MemoryFile, id: &str) -> Result<&'a MemoryEntry> {
    memory
        .items
        .iter()
        .find(|e| e.item.id == id)
        .ok_or_else(|| anyhow::anyhow!("no memory entry with id {id:?}"))
}

fn emit(memory: &MemoryFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(memory)?,
        OutputFormat::Json => serde_json::to_string_pretty(memory)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub title: String,
    pub body: String,
    pub authored_at: Option<String>,
    pub authored_in: Option<String>,
    pub attribution: Option<String>,
}

pub fn run_add(plan_dir: &Path, req: &AddRequest) -> Result<()> {
    let mut memory = read_memory(plan_dir)?;
    let id = allocate_id(&req.title, memory.items.iter().map(|e| e.item.id.as_str()));
    let entry = MemoryEntry {
        item: Item {
            id,
            kind: KindMarker::new(),
            claim: req.title.clone(),
            justifications: vec![Justification::Rationale {
                text: ensure_trailing_newline(&req.body),
            }],
            status: MemoryStatus::Active,
            supersedes: vec![],
            superseded_by: None,
            defeated_by: None,
            authored_at: req.authored_at.clone().unwrap_or_else(current_utc_rfc3339),
            authored_in: req.authored_in.clone().unwrap_or_else(|| "unspecified".into()),
        },
        attribution: req.attribution.clone(),
    };
    memory.items.push(entry);
    write_memory(plan_dir, &memory)
}

pub fn run_init(plan_dir: &Path, seed: &MemoryFile) -> Result<()> {
    let existing = read_memory(plan_dir)?;
    if !existing.items.is_empty() {
        bail!(
            "refusing to init: memory.yaml at {} is non-empty ({} entries). Use `add` for incremental inserts.",
            plan_dir.display(),
            existing.items.len()
        );
    }
    write_memory(plan_dir, seed)
}

pub fn run_set_body(plan_dir: &Path, id: &str, body: &str) -> Result<()> {
    let mut memory = read_memory(plan_dir)?;
    let entry = memory
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| anyhow::anyhow!("no memory entry with id {id:?}"))?;
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
    write_memory(plan_dir, &memory)
}

pub fn run_set_title(plan_dir: &Path, id: &str, new_title: &str) -> Result<()> {
    let mut memory = read_memory(plan_dir)?;
    let entry = memory
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| anyhow::anyhow!("no memory entry with id {id:?}"))?;
    entry.item.claim = new_title.to_string();
    write_memory(plan_dir, &memory)
}

pub fn run_set_status(plan_dir: &Path, id: &str, status_str: &str) -> Result<()> {
    let new_status = MemoryStatus::parse(status_str).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown memory status {status_str:?}; expected one of `active`, `defeated`, `superseded`"
        )
    })?;
    let mut memory = read_memory(plan_dir)?;
    let entry = memory
        .items
        .iter_mut()
        .find(|e| e.item.id == id)
        .ok_or_else(|| anyhow::anyhow!("no memory entry with id {id:?}"))?;
    let from = entry.item.status;
    if from == new_status {
        return Ok(());
    }
    let legal = MemoryStatus::transitions()
        .iter()
        .any(|(f, t)| *f == from && *t == new_status);
    if !legal {
        bail!(
            "illegal status transition for memory entry {id:?}: `{}` → `{}` is not in the transition table",
            from.as_str(),
            new_status.as_str()
        );
    }
    entry.item.status = new_status;
    write_memory(plan_dir, &memory)
}

pub fn run_delete(plan_dir: &Path, id: &str) -> Result<()> {
    let mut memory = read_memory(plan_dir)?;
    let before = memory.items.len();
    memory.items.retain(|e| e.item.id != id);
    if memory.items.len() == before {
        bail!("no memory entry with id {id:?}");
    }
    write_memory(plan_dir, &memory)
}

fn ensure_trailing_newline(body: &str) -> String {
    if body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}\n")
    }
}

/// Render the current UTC time as RFC-3339 (`YYYY-MM-DDTHH:MM:SSZ`).
/// Mirrors `discover::stage1::current_utc_rfc3339` — duplicated rather
/// than cross-module-imported because the surface is small and the
/// alternative would couple the memory CRUD path to the discover
/// pipeline. Promote to a shared module if a third call site appears.
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

    fn sample_entry(id: &str, claim: &str, rationale: &str) -> MemoryEntry {
        MemoryEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: claim.into(),
                justifications: vec![Justification::Rationale {
                    text: rationale.into(),
                }],
                status: MemoryStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            attribution: None,
        }
    }

    fn sample_memory() -> MemoryFile {
        MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![
                sample_entry("foo", "Foo claim", "Foo body.\n"),
                sample_entry("bar", "Bar claim", "Bar body.\n"),
            ],
        }
    }

    fn add_request(title: &str, body: &str) -> AddRequest {
        AddRequest {
            title: title.into(),
            body: body.into(),
            authored_at: Some("2026-04-29T00:00:00Z".into()),
            authored_in: Some("test".into()),
            attribution: None,
        }
    }

    #[test]
    fn find_entry_returns_entry_by_id() {
        let memory = sample_memory();
        let entry = find_entry(&memory, "bar").unwrap();
        assert_eq!(entry.item.id, "bar");
    }

    #[test]
    fn find_entry_errors_when_id_not_found() {
        let memory = sample_memory();
        let err = find_entry(&memory, "nonexistent").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must include bad id: {msg}");
    }

    #[test]
    fn run_add_appends_entry_with_allocated_id() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        run_add(tmp.path(), &add_request("New Entry", "Body of new entry.\n")).unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        assert_eq!(updated.items.last().unwrap().item.id, "new-entry");
        assert_eq!(updated.items.last().unwrap().item.claim, "New Entry");
        assert_eq!(updated.items.last().unwrap().item.status, MemoryStatus::Active);
    }

    #[test]
    fn run_add_records_rationale_as_first_justification() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &MemoryFile::default()).unwrap();

        run_add(tmp.path(), &add_request("Title", "Rationale text.\n")).unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        let entry = updated.items.last().unwrap();
        assert_eq!(entry.item.justifications.len(), 1);
        match &entry.item.justifications[0] {
            Justification::Rationale { text } => assert_eq!(text, "Rationale text.\n"),
            other => panic!("expected Rationale justification, got {other:?}"),
        }
    }

    #[test]
    fn run_add_propagates_attribution_when_provided() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &MemoryFile::default()).unwrap();

        let mut req = add_request("Title", "Body.\n");
        req.attribution = Some("atlas:atlas-core".into());
        run_add(tmp.path(), &req).unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        assert_eq!(
            updated.items[0].attribution.as_deref(),
            Some("atlas:atlas-core")
        );
    }

    #[test]
    fn run_init_populates_empty_memory() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &MemoryFile::default()).unwrap();

        run_init(tmp.path(), &sample_memory()).unwrap();

        let stored = read_memory(tmp.path()).unwrap();
        assert_eq!(stored.items.len(), 2);
    }

    #[test]
    fn run_init_refuses_to_overwrite_non_empty_memory() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        let err = run_init(tmp.path(), &sample_memory()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("non-empty"), "error must explain refusal: {msg}");
    }

    #[test]
    fn run_set_body_rewrites_first_rationale_justification() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        run_set_body(tmp.path(), "foo", "Rewritten body.\n").unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        match &foo.item.justifications[0] {
            Justification::Rationale { text } => assert_eq!(text, "Rewritten body.\n"),
            other => panic!("expected Rationale justification, got {other:?}"),
        }
    }

    #[test]
    fn run_set_body_inserts_rationale_when_none_exists() {
        let tmp = TempDir::new().unwrap();
        let mut memory = MemoryFile::default();
        memory.items.push(MemoryEntry {
            item: Item {
                id: "no-rationale".into(),
                kind: KindMarker::new(),
                claim: "Claim".into(),
                justifications: vec![],
                status: MemoryStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "t".into(),
                authored_in: "test".into(),
            },
            attribution: None,
        });
        write_memory(tmp.path(), &memory).unwrap();

        run_set_body(tmp.path(), "no-rationale", "First body.\n").unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        let entry = updated.items.iter().find(|e| e.item.id == "no-rationale").unwrap();
        assert_eq!(entry.item.justifications.len(), 1);
        match &entry.item.justifications[0] {
            Justification::Rationale { text } => assert_eq!(text, "First body.\n"),
            other => panic!("expected Rationale justification, got {other:?}"),
        }
    }

    #[test]
    fn run_set_title_updates_claim_but_preserves_id() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        run_set_title(tmp.path(), "bar", "Bar's New Claim").unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        let bar = updated.items.iter().find(|e| e.item.id == "bar").unwrap();
        assert_eq!(bar.item.claim, "Bar's New Claim");
        assert_eq!(bar.item.id, "bar", "id must not change when claim changes");
    }

    #[test]
    fn run_set_status_accepts_legal_transition() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        run_set_status(tmp.path(), "foo", "defeated").unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        assert_eq!(foo.item.status, MemoryStatus::Defeated);
    }

    #[test]
    fn run_set_status_rejects_unknown_status_string() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        let err = run_set_status(tmp.path(), "foo", "ghost").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ghost"), "error must cite the unknown value: {msg}");
        assert!(msg.contains("active"), "error must list legal values: {msg}");
    }

    #[test]
    fn run_set_status_rejects_illegal_transition() {
        let tmp = TempDir::new().unwrap();
        let mut memory = sample_memory();
        memory.items[0].item.status = MemoryStatus::Defeated;
        write_memory(tmp.path(), &memory).unwrap();

        // Defeated is terminal; can't go back to Active.
        let err = run_set_status(tmp.path(), "foo", "active").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("illegal"), "error must say illegal: {msg}");
        assert!(msg.contains("defeated"), "error must show from-state: {msg}");
        assert!(msg.contains("active"), "error must show to-state: {msg}");
    }

    #[test]
    fn run_set_status_self_transition_is_a_noop() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        // active → active: legal no-op.
        run_set_status(tmp.path(), "foo", "active").unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        let foo = updated.items.iter().find(|e| e.item.id == "foo").unwrap();
        assert_eq!(foo.item.status, MemoryStatus::Active);
    }

    #[test]
    fn run_delete_removes_entry_by_id() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        run_delete(tmp.path(), "foo").unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        assert_eq!(updated.items.len(), 1);
        assert_eq!(updated.items[0].item.id, "bar");
    }

    #[test]
    fn run_delete_errors_on_unknown_id() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        let err = run_delete(tmp.path(), "nonexistent").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must cite the bad id: {msg}");
    }

    #[test]
    fn format_unix_utc_renders_known_epoch_zero() {
        assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn format_unix_utc_handles_leap_year_february() {
        // 2024-02-29T00:00:00Z = 1709164800
        assert_eq!(format_unix_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    }
}
