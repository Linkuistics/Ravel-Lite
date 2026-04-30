//! Read-only cross-kind queries over a plan's knowledge graph.
//!
//! The four typed stores (`IntentsFile`, `BacklogFile`, `MemoryFile`,
//! and — once it exists — a `FindingsFile`) each own a per-kind CRUD
//! surface in `state::<kind>::verbs`. This module is the *cross-kind*
//! read surface: list all items in a plan, find one by id without
//! having to guess its kind, filter by status across kinds, or filter
//! by justification kind across kinds.
//!
//! Every verb here is read-only. Mutations stay in their per-kind
//! homes (`state backlog set-status`, `state intents set-status`,
//! `state memory set-status`). Datalog evaluation is deferred —
//! today's filters are imperative, see the
//! `datalog-evaluator-deferred-imperative-verbs-sufficient` memory.
//!
//! Backed by `docs/architecture-next.md` §CLI surface for queries.
//!
//! Wired into `main.rs` under the top-level `plan` subcommand:
//!
//! - `ravel-lite plan list-items <plan-dir> [--kind K]`
//! - `ravel-lite plan show-item <plan-dir> <id>`
//! - `ravel-lite plan query-by-status <plan-dir> --kind K --status S`
//! - `ravel-lite plan query-by-justification <plan-dir> --kind K --justification-kind J`

use std::path::Path;

use anyhow::{bail, Result};
use serde::Serialize;

use knowledge_graph::{ItemStatus, Justification};

use crate::plan_kg::{BacklogStatus, FindingStatus, IntentStatus, MemoryStatus};
use crate::state::backlog::schema::{BacklogEntry, BacklogFile};
use crate::state::backlog::yaml_io::read_backlog;
use crate::state::findings::schema::{FindingEntry, FindingsFile, FINDINGS_SCHEMA_VERSION};
use crate::state::findings::yaml_io::read_findings;
use crate::state::intents::schema::{IntentEntry, IntentsFile};
use crate::state::intents::yaml_io::read_intents;
use crate::state::memory::schema::{MemoryEntry, MemoryFile};
use crate::state::memory::yaml_io::read_memory;

// -- Kind selector -----------------------------------------------------

/// The kinds the plan-inspect verbs know how to query. Mirrors the
/// `KIND_STR` vocabulary in `plan_kg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanItemKind {
    Intent,
    BacklogItem,
    MemoryEntry,
    Finding,
}

impl PlanItemKind {
    pub fn parse(input: &str) -> Result<PlanItemKind> {
        match input {
            "intent" => Ok(PlanItemKind::Intent),
            "backlog-item" => Ok(PlanItemKind::BacklogItem),
            "memory-entry" => Ok(PlanItemKind::MemoryEntry),
            "finding" => Ok(PlanItemKind::Finding),
            other => bail!(
                "unknown --kind {other:?}; expected one of `intent`, `backlog-item`, `memory-entry`, `finding`"
            ),
        }
    }
}

// -- Justification kind filter -----------------------------------------

/// One of the six `Justification` variants. Kept separate from
/// `Justification` itself because it carries no data — it's purely a
/// shape selector for `query-by-justification`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustificationKindFilter {
    CodeAnchor,
    Rationale,
    ServesIntent,
    Defeats,
    Supersedes,
    External,
}

impl JustificationKindFilter {
    pub fn parse(input: &str) -> Result<JustificationKindFilter> {
        match input {
            "code-anchor" => Ok(JustificationKindFilter::CodeAnchor),
            "rationale" => Ok(JustificationKindFilter::Rationale),
            "serves-intent" => Ok(JustificationKindFilter::ServesIntent),
            "defeats" => Ok(JustificationKindFilter::Defeats),
            "supersedes" => Ok(JustificationKindFilter::Supersedes),
            "external" => Ok(JustificationKindFilter::External),
            other => bail!(
                "unknown --justification-kind {other:?}; expected one of `code-anchor`, `rationale`, `serves-intent`, `defeats`, `supersedes`, `external`"
            ),
        }
    }

    /// Does this filter match the given justification?
    pub fn matches(self, j: &Justification) -> bool {
        matches!(
            (self, j),
            (JustificationKindFilter::CodeAnchor, Justification::CodeAnchor { .. })
                | (JustificationKindFilter::Rationale, Justification::Rationale { .. })
                | (JustificationKindFilter::ServesIntent, Justification::ServesIntent { .. })
                | (JustificationKindFilter::Defeats, Justification::Defeats { .. })
                | (JustificationKindFilter::Supersedes, Justification::Supersedes { .. })
                | (JustificationKindFilter::External, Justification::External { .. })
        )
    }
}

// -- Cross-kind output sum type ----------------------------------------

/// One entry from any of the typed plan stores. `#[serde(untagged)]`
/// because each variant's `KindMarker<K>` already enforces the on-wire
/// `kind:` discriminant — adding a second tag would duplicate it.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum AnyEntry {
    Intent(IntentEntry),
    Backlog(BacklogEntry),
    Memory(MemoryEntry),
    Finding(FindingEntry),
}

impl AnyEntry {
    pub fn id(&self) -> &str {
        match self {
            AnyEntry::Intent(e) => &e.item.id,
            AnyEntry::Backlog(e) => &e.item.id,
            AnyEntry::Memory(e) => &e.item.id,
            AnyEntry::Finding(e) => &e.item.id,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            AnyEntry::Intent(_) => "intent",
            AnyEntry::Backlog(_) => "backlog-item",
            AnyEntry::Memory(_) => "memory-entry",
            AnyEntry::Finding(_) => "finding",
        }
    }
}

/// What `list-items` and the two `query-by-*` verbs emit when no
/// `--kind` is supplied: a single `items:` list spanning every
/// available kind. Mirrors the per-kind `*File` shape so callers parse
/// the same way regardless of whether they filtered.
#[derive(Debug, Serialize)]
pub struct AnyItemsFile {
    pub items: Vec<AnyEntry>,
}

// -- Output format ------------------------------------------------------

/// Mirrors `state::intents::OutputFormat`. The narrower vocabulary is
/// deliberate: `markdown` is the backlog-list verb's special case, and
/// the inspect verbs aren't trying to reinvent it.
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

// -- Verb: list-items ---------------------------------------------------

pub fn run_list_items(
    plan_dir: &Path,
    kind: Option<PlanItemKind>,
    format: OutputFormat,
) -> Result<()> {
    match kind {
        Some(PlanItemKind::Intent) => emit(&read_intents(plan_dir)?, format),
        Some(PlanItemKind::BacklogItem) => emit(&read_backlog(plan_dir)?, format),
        Some(PlanItemKind::MemoryEntry) => emit(&read_memory(plan_dir)?, format),
        Some(PlanItemKind::Finding) => emit(&read_findings(plan_dir)?, format),
        None => {
            let items = collect_all_entries(plan_dir)?;
            emit(&AnyItemsFile { items }, format)
        }
    }
}

// -- Verb: show-item ----------------------------------------------------

pub fn run_show_item(plan_dir: &Path, id: &str, format: OutputFormat) -> Result<()> {
    let mut hits: Vec<AnyEntry> = Vec::new();
    if let Some(e) = find_in_intents(&read_intents(plan_dir)?, id) {
        hits.push(AnyEntry::Intent(e));
    }
    if let Some(e) = find_in_backlog(&read_backlog(plan_dir)?, id) {
        hits.push(AnyEntry::Backlog(e));
    }
    if let Some(e) = find_in_memory(&read_memory(plan_dir)?, id) {
        hits.push(AnyEntry::Memory(e));
    }
    if let Some(e) = find_in_findings(&read_findings(plan_dir)?, id) {
        hits.push(AnyEntry::Finding(e));
    }
    match hits.len() {
        0 => bail!(
            "no item with id {id:?} in any of intents.yaml, backlog.yaml, memory.yaml, findings.yaml"
        ),
        1 => emit(&hits.into_iter().next().unwrap(), format),
        n => {
            let kinds: Vec<&'static str> = hits.iter().map(|e| e.kind_str()).collect();
            bail!(
                "id {id:?} is ambiguous: {n} matches across kinds {kinds:?}. \
                 Use the per-kind verb (`state intents show`, `state backlog show`, \
                 `state memory show`, `state findings show`) to disambiguate."
            )
        }
    }
}

// -- Verb: query-by-status ----------------------------------------------

pub fn run_query_by_status(
    plan_dir: &Path,
    kind: Option<PlanItemKind>,
    status: &str,
    format: OutputFormat,
) -> Result<()> {
    match kind {
        Some(PlanItemKind::Intent) => {
            let s = parse_intent_status(status)?;
            let file = read_intents(plan_dir)?;
            let filtered = filter_intents_by_status(&file, s);
            emit(&intents_file_with(filtered), format)
        }
        Some(PlanItemKind::BacklogItem) => {
            let s = parse_backlog_status(status)?;
            let file = read_backlog(plan_dir)?;
            let filtered = filter_backlog_by_status(&file, s);
            emit(&backlog_file_with(filtered), format)
        }
        Some(PlanItemKind::MemoryEntry) => {
            let s = parse_memory_status(status)?;
            let file = read_memory(plan_dir)?;
            let filtered = filter_memory_by_status(&file, s);
            emit(&memory_file_with(filtered), format)
        }
        Some(PlanItemKind::Finding) => {
            let s = parse_finding_status(status)?;
            let file = read_findings(plan_dir)?;
            let filtered = filter_findings_by_status(&file, s);
            emit(&findings_file_with(filtered), format)
        }
        None => {
            // Cross-kind: status must match at least one kind's vocabulary.
            // Match against any kind whose status enum accepts the string.
            let mut hits: Vec<AnyEntry> = Vec::new();
            if let Some(s) = IntentStatus::parse(status) {
                let file = read_intents(plan_dir)?;
                hits.extend(
                    filter_intents_by_status(&file, s)
                        .into_iter()
                        .map(AnyEntry::Intent),
                );
            }
            if let Some(s) = BacklogStatus::parse(status) {
                let file = read_backlog(plan_dir)?;
                hits.extend(
                    filter_backlog_by_status(&file, s)
                        .into_iter()
                        .map(AnyEntry::Backlog),
                );
            }
            if let Some(s) = MemoryStatus::parse(status) {
                let file = read_memory(plan_dir)?;
                hits.extend(
                    filter_memory_by_status(&file, s)
                        .into_iter()
                        .map(AnyEntry::Memory),
                );
            }
            if let Some(s) = FindingStatus::parse(status) {
                let file = read_findings(plan_dir)?;
                hits.extend(
                    filter_findings_by_status(&file, s)
                        .into_iter()
                        .map(AnyEntry::Finding),
                );
            }
            if hits.is_empty() && !any_kind_accepts_status(status) {
                bail!(
                    "status {status:?} is not a member of any kind's vocabulary. \
                     Intent: {:?}; backlog-item: {:?}; memory-entry: {:?}; finding: {:?}.",
                    intent_status_words(),
                    backlog_status_words(),
                    memory_status_words(),
                    finding_status_words(),
                );
            }
            emit(&AnyItemsFile { items: hits }, format)
        }
    }
}

// -- Verb: query-by-justification ---------------------------------------

pub fn run_query_by_justification(
    plan_dir: &Path,
    kind: Option<PlanItemKind>,
    jk: JustificationKindFilter,
    format: OutputFormat,
) -> Result<()> {
    match kind {
        Some(PlanItemKind::Intent) => {
            let file = read_intents(plan_dir)?;
            let filtered = filter_intents_by_justification(&file, jk);
            emit(&intents_file_with(filtered), format)
        }
        Some(PlanItemKind::BacklogItem) => {
            let file = read_backlog(plan_dir)?;
            let filtered = filter_backlog_by_justification(&file, jk);
            emit(&backlog_file_with(filtered), format)
        }
        Some(PlanItemKind::MemoryEntry) => {
            let file = read_memory(plan_dir)?;
            let filtered = filter_memory_by_justification(&file, jk);
            emit(&memory_file_with(filtered), format)
        }
        Some(PlanItemKind::Finding) => {
            let file = read_findings(plan_dir)?;
            let filtered = filter_findings_by_justification(&file, jk);
            emit(&findings_file_with(filtered), format)
        }
        None => {
            let mut hits: Vec<AnyEntry> = Vec::new();
            let intents = read_intents(plan_dir)?;
            hits.extend(
                filter_intents_by_justification(&intents, jk)
                    .into_iter()
                    .map(AnyEntry::Intent),
            );
            let backlog = read_backlog(plan_dir)?;
            hits.extend(
                filter_backlog_by_justification(&backlog, jk)
                    .into_iter()
                    .map(AnyEntry::Backlog),
            );
            let memory = read_memory(plan_dir)?;
            hits.extend(
                filter_memory_by_justification(&memory, jk)
                    .into_iter()
                    .map(AnyEntry::Memory),
            );
            let findings = read_findings(plan_dir)?;
            hits.extend(
                filter_findings_by_justification(&findings, jk)
                    .into_iter()
                    .map(AnyEntry::Finding),
            );
            emit(&AnyItemsFile { items: hits }, format)
        }
    }
}

// -- Pure filter functions (unit-testable without disk) -----------------

pub fn filter_intents_by_status(file: &IntentsFile, status: IntentStatus) -> Vec<IntentEntry> {
    file.items
        .iter()
        .filter(|e| e.item.status == status)
        .cloned()
        .collect()
}

pub fn filter_backlog_by_status(file: &BacklogFile, status: BacklogStatus) -> Vec<BacklogEntry> {
    file.items
        .iter()
        .filter(|e| e.item.status == status)
        .cloned()
        .collect()
}

pub fn filter_memory_by_status(file: &MemoryFile, status: MemoryStatus) -> Vec<MemoryEntry> {
    file.items
        .iter()
        .filter(|e| e.item.status == status)
        .cloned()
        .collect()
}

pub fn filter_intents_by_justification(
    file: &IntentsFile,
    jk: JustificationKindFilter,
) -> Vec<IntentEntry> {
    file.items
        .iter()
        .filter(|e| e.item.justifications.iter().any(|j| jk.matches(j)))
        .cloned()
        .collect()
}

pub fn filter_backlog_by_justification(
    file: &BacklogFile,
    jk: JustificationKindFilter,
) -> Vec<BacklogEntry> {
    file.items
        .iter()
        .filter(|e| e.item.justifications.iter().any(|j| jk.matches(j)))
        .cloned()
        .collect()
}

pub fn filter_memory_by_justification(
    file: &MemoryFile,
    jk: JustificationKindFilter,
) -> Vec<MemoryEntry> {
    file.items
        .iter()
        .filter(|e| e.item.justifications.iter().any(|j| jk.matches(j)))
        .cloned()
        .collect()
}

pub fn filter_findings_by_status(
    file: &FindingsFile,
    status: FindingStatus,
) -> Vec<FindingEntry> {
    file.items
        .iter()
        .filter(|e| e.item.status == status)
        .cloned()
        .collect()
}

pub fn filter_findings_by_justification(
    file: &FindingsFile,
    jk: JustificationKindFilter,
) -> Vec<FindingEntry> {
    file.items
        .iter()
        .filter(|e| e.item.justifications.iter().any(|j| jk.matches(j)))
        .cloned()
        .collect()
}

// -- Lookup helpers -----------------------------------------------------

fn find_in_intents(file: &IntentsFile, id: &str) -> Option<IntentEntry> {
    file.items.iter().find(|e| e.item.id == id).cloned()
}

fn find_in_backlog(file: &BacklogFile, id: &str) -> Option<BacklogEntry> {
    file.items.iter().find(|e| e.item.id == id).cloned()
}

fn find_in_memory(file: &MemoryFile, id: &str) -> Option<MemoryEntry> {
    file.items.iter().find(|e| e.item.id == id).cloned()
}

fn find_in_findings(file: &FindingsFile, id: &str) -> Option<FindingEntry> {
    file.items.iter().find(|e| e.item.id == id).cloned()
}

fn collect_all_entries(plan_dir: &Path) -> Result<Vec<AnyEntry>> {
    let intents = read_intents(plan_dir)?;
    let backlog = read_backlog(plan_dir)?;
    let memory = read_memory(plan_dir)?;
    let findings = read_findings(plan_dir)?;
    let mut items: Vec<AnyEntry> = Vec::with_capacity(
        intents.items.len()
            + backlog.items.len()
            + memory.items.len()
            + findings.items.len(),
    );
    items.extend(intents.items.into_iter().map(AnyEntry::Intent));
    items.extend(backlog.items.into_iter().map(AnyEntry::Backlog));
    items.extend(memory.items.into_iter().map(AnyEntry::Memory));
    items.extend(findings.items.into_iter().map(AnyEntry::Finding));
    Ok(items)
}

// -- File-shape wrappers (preserve the per-kind on-wire shape) ---------

fn intents_file_with(items: Vec<IntentEntry>) -> IntentsFile {
    IntentsFile {
        schema_version: crate::state::intents::INTENTS_SCHEMA_VERSION,
        items,
    }
}

fn backlog_file_with(items: Vec<BacklogEntry>) -> BacklogFile {
    BacklogFile {
        schema_version: crate::state::backlog::schema::BACKLOG_SCHEMA_VERSION,
        items,
    }
}

fn memory_file_with(items: Vec<MemoryEntry>) -> MemoryFile {
    MemoryFile {
        schema_version: crate::state::memory::schema::MEMORY_SCHEMA_VERSION,
        items,
    }
}

fn findings_file_with(items: Vec<FindingEntry>) -> FindingsFile {
    FindingsFile {
        schema_version: FINDINGS_SCHEMA_VERSION,
        items,
    }
}

// -- Status-string parsing with helpful errors -------------------------

fn parse_intent_status(input: &str) -> Result<IntentStatus> {
    IntentStatus::parse(input).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown intent status {input:?}; expected one of {:?}",
            intent_status_words(),
        )
    })
}

fn parse_backlog_status(input: &str) -> Result<BacklogStatus> {
    BacklogStatus::parse(input).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown backlog-item status {input:?}; expected one of {:?}",
            backlog_status_words(),
        )
    })
}

fn parse_memory_status(input: &str) -> Result<MemoryStatus> {
    MemoryStatus::parse(input).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown memory-entry status {input:?}; expected one of {:?}",
            memory_status_words(),
        )
    })
}

fn parse_finding_status(input: &str) -> Result<FindingStatus> {
    FindingStatus::parse(input).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown finding status {input:?}; expected one of {:?}",
            finding_status_words(),
        )
    })
}

fn any_kind_accepts_status(input: &str) -> bool {
    IntentStatus::parse(input).is_some()
        || BacklogStatus::parse(input).is_some()
        || MemoryStatus::parse(input).is_some()
        || FindingStatus::parse(input).is_some()
}

fn intent_status_words() -> Vec<&'static str> {
    [
        IntentStatus::Active,
        IntentStatus::Satisfied,
        IntentStatus::Defeated,
        IntentStatus::Superseded,
    ]
    .iter()
    .map(|s| s.as_str())
    .collect()
}

fn backlog_status_words() -> Vec<&'static str> {
    [
        BacklogStatus::Active,
        BacklogStatus::Done,
        BacklogStatus::Defeated,
        BacklogStatus::Superseded,
        BacklogStatus::Blocked,
    ]
    .iter()
    .map(|s| s.as_str())
    .collect()
}

fn memory_status_words() -> Vec<&'static str> {
    [
        MemoryStatus::Active,
        MemoryStatus::Defeated,
        MemoryStatus::Superseded,
    ]
    .iter()
    .map(|s| s.as_str())
    .collect()
}

fn finding_status_words() -> Vec<&'static str> {
    [
        FindingStatus::New,
        FindingStatus::Promoted,
        FindingStatus::Wontfix,
        FindingStatus::Superseded,
    ]
    .iter()
    .map(|s| s.as_str())
    .collect()
}

// -- Emit ---------------------------------------------------------------

fn emit<T: Serialize>(value: &T, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(value)?,
        OutputFormat::Json => serde_json::to_string_pretty(value)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use knowledge_graph::{Item, KindMarker};
    use tempfile::TempDir;

    use crate::plan_kg::FindingStatus;
    use crate::state::backlog::schema::BACKLOG_SCHEMA_VERSION;
    use crate::state::backlog::yaml_io::write_backlog;
    use crate::state::findings::yaml_io::write_findings;
    use crate::state::intents::yaml_io::write_intents;
    use crate::state::intents::INTENTS_SCHEMA_VERSION;
    use crate::state::memory::schema::MEMORY_SCHEMA_VERSION;
    use crate::state::memory::yaml_io::write_memory;

    // -- Fixture builders ---------------------------------------------

    fn intent(id: &str, status: IntentStatus, justs: Vec<Justification>) -> IntentEntry {
        IntentEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: format!("intent {id}"),
                justifications: justs,
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
        }
    }

    fn backlog(id: &str, status: BacklogStatus, justs: Vec<Justification>) -> BacklogEntry {
        BacklogEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: format!("backlog {id}"),
                justifications: justs,
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            category: "test".into(),
            blocked_reason: None,
            dependencies: vec![],
            results: None,
            handoff: None,
        }
    }

    fn memory(id: &str, status: MemoryStatus, justs: Vec<Justification>) -> MemoryEntry {
        MemoryEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: format!("memory {id}"),
                justifications: justs,
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            attribution: None,
        }
    }

    fn finding(id: &str, status: FindingStatus, justs: Vec<Justification>) -> FindingEntry {
        FindingEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: format!("finding {id}"),
                justifications: justs,
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            component: None,
            raised_in: None,
        }
    }

    fn rationale(text: &str) -> Justification {
        Justification::Rationale {
            text: text.into(),
        }
    }

    fn serves(id: &str) -> Justification {
        Justification::ServesIntent {
            intent_id: id.into(),
        }
    }

    fn anchor(path: &str) -> Justification {
        Justification::CodeAnchor {
            component: "test:test".into(),
            path: path.into(),
            lines: None,
            sha_at_assertion: "deadbeef".into(),
        }
    }

    fn defeats(id: &str) -> Justification {
        Justification::Defeats {
            item_id: id.into(),
        }
    }

    /// Write a fully-populated plan dir with all three on-disk stores
    /// so the disk-touching verbs have something realistic to read.
    fn write_full_plan(tmp: &Path) {
        write_intents(
            tmp,
            &IntentsFile {
                schema_version: INTENTS_SCHEMA_VERSION,
                items: vec![
                    intent("i-a", IntentStatus::Active, vec![rationale("plan-A.\n")]),
                    intent("i-b", IntentStatus::Satisfied, vec![rationale("plan-B.\n")]),
                ],
            },
        )
        .unwrap();
        write_backlog(
            tmp,
            &BacklogFile {
                schema_version: BACKLOG_SCHEMA_VERSION,
                items: vec![
                    backlog("t-1", BacklogStatus::Active, vec![rationale("body.\n"), serves("i-a")]),
                    backlog("t-2", BacklogStatus::Done, vec![rationale("body.\n")]),
                    backlog("t-3", BacklogStatus::Blocked, vec![defeats("t-2")]),
                ],
            },
        )
        .unwrap();
        write_memory(
            tmp,
            &MemoryFile {
                schema_version: MEMORY_SCHEMA_VERSION,
                items: vec![
                    memory("m-x", MemoryStatus::Active, vec![anchor("src/lib.rs")]),
                    memory("m-y", MemoryStatus::Defeated, vec![rationale("note.\n")]),
                ],
            },
        )
        .unwrap();
        write_findings(
            tmp,
            &FindingsFile {
                schema_version: FINDINGS_SCHEMA_VERSION,
                items: vec![
                    finding("f-1", FindingStatus::New, vec![rationale("observed.\n")]),
                    finding(
                        "f-2",
                        FindingStatus::Promoted,
                        vec![rationale("observed.\n"), serves("i-a")],
                    ),
                ],
            },
        )
        .unwrap();
    }

    // -- Pure filters --------------------------------------------------

    #[test]
    fn filter_intents_by_status_picks_only_matching() {
        let file = IntentsFile {
            schema_version: INTENTS_SCHEMA_VERSION,
            items: vec![
                intent("a", IntentStatus::Active, vec![]),
                intent("b", IntentStatus::Satisfied, vec![]),
                intent("c", IntentStatus::Active, vec![]),
            ],
        };
        let active = filter_intents_by_status(&file, IntentStatus::Active);
        assert_eq!(active.len(), 2);
        assert_eq!(active[0].item.id, "a");
        assert_eq!(active[1].item.id, "c");
    }

    #[test]
    fn filter_backlog_by_status_picks_blocked() {
        let file = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                backlog("a", BacklogStatus::Active, vec![]),
                backlog("b", BacklogStatus::Blocked, vec![]),
                backlog("c", BacklogStatus::Done, vec![]),
            ],
        };
        let blocked = filter_backlog_by_status(&file, BacklogStatus::Blocked);
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].item.id, "b");
    }

    #[test]
    fn filter_memory_by_status_excludes_terminal() {
        let file = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![
                memory("a", MemoryStatus::Active, vec![]),
                memory("b", MemoryStatus::Defeated, vec![]),
                memory("c", MemoryStatus::Superseded, vec![]),
            ],
        };
        let active = filter_memory_by_status(&file, MemoryStatus::Active);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].item.id, "a");
    }

    #[test]
    fn justification_kind_matches_each_variant() {
        let cases = [
            (
                JustificationKindFilter::CodeAnchor,
                Justification::CodeAnchor {
                    component: "x".into(),
                    path: "p".into(),
                    lines: None,
                    sha_at_assertion: "s".into(),
                },
            ),
            (
                JustificationKindFilter::Rationale,
                Justification::Rationale { text: "r".into() },
            ),
            (
                JustificationKindFilter::ServesIntent,
                Justification::ServesIntent {
                    intent_id: "i".into(),
                },
            ),
            (
                JustificationKindFilter::Defeats,
                Justification::Defeats { item_id: "d".into() },
            ),
            (
                JustificationKindFilter::Supersedes,
                Justification::Supersedes { item_id: "s".into() },
            ),
            (
                JustificationKindFilter::External,
                Justification::External {
                    uri: "u".into(),
                },
            ),
        ];
        for (filter, j) in &cases {
            assert!(filter.matches(j), "{filter:?} should match {j:?}");
        }
    }

    #[test]
    fn justification_kind_filter_does_not_match_other_variants() {
        let r = Justification::Rationale { text: "x".into() };
        assert!(!JustificationKindFilter::CodeAnchor.matches(&r));
        assert!(!JustificationKindFilter::ServesIntent.matches(&r));
    }

    #[test]
    fn filter_backlog_by_justification_serves_intent() {
        let file = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                backlog("a", BacklogStatus::Active, vec![rationale("body.\n")]),
                backlog(
                    "b",
                    BacklogStatus::Active,
                    vec![rationale("body.\n"), serves("i-1")],
                ),
                backlog("c", BacklogStatus::Active, vec![serves("i-2")]),
            ],
        };
        let hits = filter_backlog_by_justification(&file, JustificationKindFilter::ServesIntent);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].item.id, "b");
        assert_eq!(hits[1].item.id, "c");
    }

    #[test]
    fn filter_memory_by_justification_code_anchor() {
        let file = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![
                memory("a", MemoryStatus::Active, vec![rationale("note.\n")]),
                memory("b", MemoryStatus::Active, vec![anchor("src/lib.rs")]),
            ],
        };
        let hits = filter_memory_by_justification(&file, JustificationKindFilter::CodeAnchor);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.id, "b");
    }

    #[test]
    fn filter_findings_by_status_picks_only_matching() {
        let file = FindingsFile {
            schema_version: FINDINGS_SCHEMA_VERSION,
            items: vec![
                finding("a", FindingStatus::New, vec![]),
                finding("b", FindingStatus::Promoted, vec![]),
                finding("c", FindingStatus::New, vec![]),
            ],
        };
        let new = filter_findings_by_status(&file, FindingStatus::New);
        assert_eq!(new.len(), 2);
        assert_eq!(new[0].item.id, "a");
        assert_eq!(new[1].item.id, "c");
    }

    #[test]
    fn filter_findings_by_justification_serves_intent() {
        let file = FindingsFile {
            schema_version: FINDINGS_SCHEMA_VERSION,
            items: vec![
                finding("a", FindingStatus::New, vec![rationale("body.\n")]),
                finding(
                    "b",
                    FindingStatus::New,
                    vec![rationale("body.\n"), serves("i-1")],
                ),
            ],
        };
        let hits = filter_findings_by_justification(&file, JustificationKindFilter::ServesIntent);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item.id, "b");
    }

    // -- Parsers --------------------------------------------------------

    #[test]
    fn plan_item_kind_parses_each_known_kind() {
        assert_eq!(PlanItemKind::parse("intent").unwrap(), PlanItemKind::Intent);
        assert_eq!(
            PlanItemKind::parse("backlog-item").unwrap(),
            PlanItemKind::BacklogItem
        );
        assert_eq!(
            PlanItemKind::parse("memory-entry").unwrap(),
            PlanItemKind::MemoryEntry
        );
        assert_eq!(PlanItemKind::parse("finding").unwrap(), PlanItemKind::Finding);
    }

    #[test]
    fn plan_item_kind_rejects_unknown_with_helpful_message() {
        let err = PlanItemKind::parse("ghost").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ghost"), "error must echo bad input: {msg}");
        assert!(msg.contains("backlog-item"), "error must list options: {msg}");
    }

    #[test]
    fn justification_kind_filter_parses_each_known_kind() {
        for s in [
            "code-anchor",
            "rationale",
            "serves-intent",
            "defeats",
            "supersedes",
            "external",
        ] {
            JustificationKindFilter::parse(s)
                .unwrap_or_else(|e| panic!("{s} should parse: {e}"));
        }
    }

    #[test]
    fn output_format_parses_yaml_and_json() {
        assert_eq!(OutputFormat::parse("yaml"), Some(OutputFormat::Yaml));
        assert_eq!(OutputFormat::parse("json"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::parse("toml"), None);
    }

    // -- Disk-touching verbs (hermetic via TempDir) --------------------

    #[test]
    fn run_list_items_per_kind_returns_only_that_kind() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        // No assertion on stdout; just verify each kind dispatches without
        // error and the file IO succeeds.
        run_list_items(tmp.path(), Some(PlanItemKind::Intent), OutputFormat::Yaml).unwrap();
        run_list_items(tmp.path(), Some(PlanItemKind::BacklogItem), OutputFormat::Yaml).unwrap();
        run_list_items(tmp.path(), Some(PlanItemKind::MemoryEntry), OutputFormat::Yaml).unwrap();
        run_list_items(tmp.path(), Some(PlanItemKind::Finding), OutputFormat::Yaml).unwrap();
    }

    #[test]
    fn run_list_items_no_kind_returns_unified_view() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        run_list_items(tmp.path(), None, OutputFormat::Yaml).unwrap();
    }

    #[test]
    fn run_show_item_finds_id_in_any_kind() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        // i-a is an intent.
        run_show_item(tmp.path(), "i-a", OutputFormat::Yaml).unwrap();
        // t-1 is a backlog item.
        run_show_item(tmp.path(), "t-1", OutputFormat::Yaml).unwrap();
        // m-x is a memory entry.
        run_show_item(tmp.path(), "m-x", OutputFormat::Yaml).unwrap();
        // f-1 is a finding.
        run_show_item(tmp.path(), "f-1", OutputFormat::Yaml).unwrap();
    }

    #[test]
    fn run_show_item_errors_when_id_not_found() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        let err = run_show_item(tmp.path(), "nonexistent", OutputFormat::Yaml).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must echo id: {msg}");
    }

    #[test]
    fn run_show_item_errors_on_id_collision_across_kinds() {
        // Pathological case: same id `dup` exists as both an intent and
        // a backlog item. The verb refuses to guess.
        let tmp = TempDir::new().unwrap();
        write_intents(
            tmp.path(),
            &IntentsFile {
                schema_version: INTENTS_SCHEMA_VERSION,
                items: vec![intent("dup", IntentStatus::Active, vec![])],
            },
        )
        .unwrap();
        write_backlog(
            tmp.path(),
            &BacklogFile {
                schema_version: BACKLOG_SCHEMA_VERSION,
                items: vec![backlog("dup", BacklogStatus::Active, vec![])],
            },
        )
        .unwrap();
        write_memory(tmp.path(), &MemoryFile::default()).unwrap();
        let err = run_show_item(tmp.path(), "dup", OutputFormat::Yaml).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ambiguous"), "error must say ambiguous: {msg}");
        assert!(msg.contains("intent"), "error must list intent kind: {msg}");
        assert!(msg.contains("backlog-item"), "error must list backlog kind: {msg}");
    }

    #[test]
    fn run_query_by_status_per_kind_with_legal_value() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        run_query_by_status(
            tmp.path(),
            Some(PlanItemKind::BacklogItem),
            "blocked",
            OutputFormat::Yaml,
        )
        .unwrap();
    }

    #[test]
    fn run_query_by_status_per_kind_rejects_status_outside_kind_vocab() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        // `blocked` is a backlog status; intents don't have it.
        let err = run_query_by_status(
            tmp.path(),
            Some(PlanItemKind::Intent),
            "blocked",
            OutputFormat::Yaml,
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("intent status"), "error must name the kind: {msg}");
        assert!(msg.contains("blocked"), "error must echo bad input: {msg}");
    }

    #[test]
    fn run_query_by_status_cross_kind_unions_matches() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        // `active` is shared by all three kinds; the cross-kind verb
        // unions matches.
        run_query_by_status(tmp.path(), None, "active", OutputFormat::Yaml).unwrap();
    }

    #[test]
    fn run_query_by_status_cross_kind_rejects_status_no_kind_accepts() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        let err =
            run_query_by_status(tmp.path(), None, "ghost", OutputFormat::Yaml).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("ghost"),
            "error must echo bad input: {msg}"
        );
        assert!(
            msg.contains("vocabulary"),
            "error must explain it's not in any kind's vocabulary: {msg}"
        );
        // The error must also enumerate the finding vocabulary so users
        // can see all four kinds at once. Otherwise `new`/`promoted`/
        // `wontfix` look like accepted-but-empty queries.
        assert!(
            msg.contains("finding"),
            "error must list finding vocabulary: {msg}"
        );
    }

    #[test]
    fn run_query_by_status_per_kind_finding_with_legal_value() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        run_query_by_status(
            tmp.path(),
            Some(PlanItemKind::Finding),
            "new",
            OutputFormat::Yaml,
        )
        .unwrap();
    }

    #[test]
    fn run_query_by_status_per_kind_finding_rejects_status_outside_vocab() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        // `blocked` is a backlog status; findings don't have it.
        let err = run_query_by_status(
            tmp.path(),
            Some(PlanItemKind::Finding),
            "blocked",
            OutputFormat::Yaml,
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("finding status"), "error must name the kind: {msg}");
        assert!(msg.contains("blocked"), "error must echo bad input: {msg}");
    }

    #[test]
    fn run_query_by_status_cross_kind_finds_finding_only_status() {
        // `new` is in finding's vocabulary alone — no other kind accepts
        // it. The cross-kind walk must still resolve it via the finding
        // store.
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        run_query_by_status(tmp.path(), None, "new", OutputFormat::Yaml).unwrap();
    }

    #[test]
    fn run_query_by_justification_serves_intent_picks_only_intent_serving_items() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        run_query_by_justification(
            tmp.path(),
            Some(PlanItemKind::BacklogItem),
            JustificationKindFilter::ServesIntent,
            OutputFormat::Yaml,
        )
        .unwrap();
    }

    #[test]
    fn run_query_by_justification_cross_kind_walks_all_stores() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        run_query_by_justification(
            tmp.path(),
            None,
            JustificationKindFilter::CodeAnchor,
            OutputFormat::Yaml,
        )
        .unwrap();
    }

    #[test]
    fn run_query_by_justification_per_kind_finding() {
        let tmp = TempDir::new().unwrap();
        write_full_plan(tmp.path());
        run_query_by_justification(
            tmp.path(),
            Some(PlanItemKind::Finding),
            JustificationKindFilter::ServesIntent,
            OutputFormat::Yaml,
        )
        .unwrap();
    }

    // -- AnyEntry ------------------------------------------------------

    #[test]
    fn any_entry_id_and_kind_str() {
        let i = AnyEntry::Intent(intent("i-1", IntentStatus::Active, vec![]));
        let b = AnyEntry::Backlog(backlog("t-1", BacklogStatus::Active, vec![]));
        let m = AnyEntry::Memory(memory("m-1", MemoryStatus::Active, vec![]));
        let f = AnyEntry::Finding(finding("f-1", FindingStatus::New, vec![]));
        assert_eq!(i.id(), "i-1");
        assert_eq!(b.id(), "t-1");
        assert_eq!(m.id(), "m-1");
        assert_eq!(f.id(), "f-1");
        assert_eq!(i.kind_str(), "intent");
        assert_eq!(b.kind_str(), "backlog-item");
        assert_eq!(m.kind_str(), "memory-entry");
        assert_eq!(f.kind_str(), "finding");
    }

    #[test]
    fn any_entry_serialises_with_underlying_kind_marker() {
        // The untagged sum carries the kind discriminant via each
        // variant's `KindMarker<K>`. Round-trip is one-way (Serialize
        // only) but the YAML must contain the kind string.
        let i = AnyEntry::Intent(intent("i-1", IntentStatus::Active, vec![]));
        let yaml = serde_yaml::to_string(&i).unwrap();
        assert!(yaml.contains("kind: intent"), "yaml was:\n{yaml}");
    }
}
