//! migrate-memory-backfill phase application: parse
//! `migrate-memory-proposal.yaml`, set `attribution` on each memory
//! entry; entries with null attribution receive `status: legacy`.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::agent::Agent;
use crate::bail_with;
use crate::cli::ErrorCode;
use crate::plan_kg::MemoryStatus;
use crate::state::memory::yaml_io::{read_memory, write_memory};

use super::proposals::{MemoryProposal, MEMORY_PROPOSAL_FILENAME};
use super::validate::Validated;

pub async fn run(agent: Arc<dyn Agent>, v: &Validated) -> Result<()> {
    super::orchestrator::invoke_phase(
        agent,
        v,
        crate::types::LlmPhase::MigrateMemoryBackfill,
    )
    .await?;
    apply_proposal(&v.new_plan_dir)
}

pub fn apply_proposal(plan_dir: &Path) -> Result<()> {
    let scratch = plan_dir.join(MEMORY_PROPOSAL_FILENAME);
    if !scratch.is_file() {
        bail_with!(
            ErrorCode::NotFound,
            "{} not written by migrate-memory-backfill phase",
            scratch.display()
        );
    }
    let body = fs::read_to_string(&scratch)?;
    let proposal: MemoryProposal = serde_yaml::from_str(&body)?;

    let mut memory = read_memory(plan_dir)?;
    for attr in &proposal.attributions {
        let entry = memory
            .items
            .iter_mut()
            .find(|e| e.item.id == attr.entry_id);
        let Some(entry) = entry else {
            bail_with!(
                ErrorCode::InvalidInput,
                "proposal references unknown memory entry id {:?}",
                attr.entry_id
            );
        };
        match &attr.attribution {
            Some(s) => entry.attribution = Some(s.clone()),
            None => {
                entry.attribution = None;
                entry.item.status = MemoryStatus::Legacy;
            }
        }
    }
    write_memory(plan_dir, &memory)?;

    fs::remove_file(&scratch).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::memory::yaml_io::write_memory;
    use crate::state::memory::{MemoryEntry, MemoryFile, MEMORY_SCHEMA_VERSION};
    use knowledge_graph::{Item, KindMarker};
    use tempfile::TempDir;

    fn memory_with_two_entries(plan: &Path) {
        let m = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![
                MemoryEntry {
                    item: Item {
                        id: "m-001".into(),
                        kind: KindMarker::new(),
                        claim: "Lesson 1".into(),
                        justifications: vec![],
                        status: MemoryStatus::Active,
                        supersedes: vec![],
                        superseded_by: None,
                        defeated_by: None,
                        authored_at: "2026-04-01T00:00:00Z".into(),
                        authored_in: "test".into(),
                    },
                    attribution: None,
                },
                MemoryEntry {
                    item: Item {
                        id: "m-002".into(),
                        kind: KindMarker::new(),
                        claim: "Lesson 2".into(),
                        justifications: vec![],
                        status: MemoryStatus::Active,
                        supersedes: vec![],
                        superseded_by: None,
                        defeated_by: None,
                        authored_at: "2026-04-01T00:00:00Z".into(),
                        authored_in: "test".into(),
                    },
                    attribution: None,
                },
            ],
        };
        write_memory(plan, &m).unwrap();
    }

    #[test]
    fn apply_sets_attribution_and_marks_null_as_legacy() {
        let tmp = TempDir::new().unwrap();
        memory_with_two_entries(tmp.path());

        let proposal = MemoryProposal {
            attributions: vec![
                super::super::proposals::MemoryAttribution {
                    entry_id: "m-001".into(),
                    attribution: Some("atlas:atlas-core".into()),
                },
                super::super::proposals::MemoryAttribution {
                    entry_id: "m-002".into(),
                    attribution: None,
                },
            ],
        };
        fs::write(
            tmp.path().join(MEMORY_PROPOSAL_FILENAME),
            serde_yaml::to_string(&proposal).unwrap(),
        )
        .unwrap();

        apply_proposal(tmp.path()).unwrap();

        let memory = crate::state::memory::yaml_io::read_memory(tmp.path()).unwrap();
        let m1 = memory.items.iter().find(|e| e.item.id == "m-001").unwrap();
        assert_eq!(m1.attribution.as_deref(), Some("atlas:atlas-core"));
        assert_eq!(m1.item.status, MemoryStatus::Active);

        let m2 = memory.items.iter().find(|e| e.item.id == "m-002").unwrap();
        assert_eq!(m2.attribution, None);
        assert_eq!(m2.item.status, MemoryStatus::Legacy);
    }

    #[test]
    fn apply_errors_on_unknown_entry_id() {
        let tmp = TempDir::new().unwrap();
        memory_with_two_entries(tmp.path());
        let proposal = MemoryProposal {
            attributions: vec![super::super::proposals::MemoryAttribution {
                entry_id: "m-nonexistent".into(),
                attribution: None,
            }],
        };
        fs::write(
            tmp.path().join(MEMORY_PROPOSAL_FILENAME),
            serde_yaml::to_string(&proposal).unwrap(),
        )
        .unwrap();
        let err = apply_proposal(tmp.path()).unwrap_err();
        assert!(format!("{err:#}").contains("unknown memory entry id"));
    }
}
