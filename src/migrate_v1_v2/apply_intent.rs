//! migrate-intent phase application: parse `migrate-intent-proposal.yaml`,
//! write `intents.yaml`, mutate `backlog.yaml` adding either a
//! `serves-intent` justification or stamping `legacy: true`.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use knowledge_graph::Justification;

use crate::agent::Agent;
use crate::bail_with;
use crate::cli::ErrorCode;
use crate::state::backlog::yaml_io::{read_backlog, write_backlog};
use crate::state::intents::yaml_io::write_intents;
use crate::state::intents::{IntentsFile, INTENTS_SCHEMA_VERSION};

use super::proposals::{IntentProposal, INTENT_PROPOSAL_FILENAME};
use super::validate::Validated;

pub async fn run(
    agent: Arc<dyn Agent>,
    v: &Validated,
    skip_confirm: bool,
) -> Result<()> {
    super::orchestrator::invoke_phase(agent, v, crate::types::LlmPhase::MigrateIntent).await?;
    apply_proposal(&v.new_plan_dir, skip_confirm)
}

pub fn apply_proposal(plan_dir: &Path, skip_confirm: bool) -> Result<()> {
    let scratch = plan_dir.join(INTENT_PROPOSAL_FILENAME);
    if !scratch.is_file() {
        bail_with!(
            ErrorCode::NotFound,
            "{} not written by migrate-intent phase",
            scratch.display()
        );
    }
    let body = fs::read_to_string(&scratch)?;
    let proposal: IntentProposal = serde_yaml::from_str(&body)?;

    if !skip_confirm {
        super::orchestrator::confirm(&format!(
            "migrate-intent: apply {} intents and {} item attributions?",
            proposal.intents.len(),
            proposal.item_attributions.len()
        ))?;
    }

    let intents = IntentsFile {
        schema_version: INTENTS_SCHEMA_VERSION,
        items: proposal.intents.clone(),
    };
    write_intents(plan_dir, &intents)?;

    let mut backlog = read_backlog(plan_dir)?;
    for attr in &proposal.item_attributions {
        let entry = backlog.items.iter_mut().find(|e| e.item.id == attr.item_id);
        let Some(entry) = entry else {
            bail_with!(
                ErrorCode::InvalidInput,
                "proposal references unknown backlog item id {:?}",
                attr.item_id
            );
        };
        if attr.is_legacy() {
            entry.legacy = true;
        } else {
            entry.item.justifications.push(Justification::ServesIntent {
                intent_id: attr.serves.clone(),
            });
        }
    }
    write_backlog(plan_dir, &backlog)?;

    fs::remove_file(&scratch).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_kg::BacklogStatus;
    use crate::state::backlog::yaml_io::write_backlog;
    use crate::state::backlog::{BacklogEntry, BacklogFile, BACKLOG_SCHEMA_VERSION};
    use knowledge_graph::{Item, KindMarker};
    use tempfile::TempDir;

    fn backlog_with_two_items(plan: &Path) {
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![
                BacklogEntry {
                    item: Item {
                        id: "t-001".into(),
                        kind: KindMarker::new(),
                        claim: "Task 1".into(),
                        justifications: vec![],
                        status: BacklogStatus::Active,
                        supersedes: vec![],
                        superseded_by: None,
                        defeated_by: None,
                        authored_at: "2026-04-01T00:00:00Z".into(),
                        authored_in: "test".into(),
                    },
                    category: "x".into(),
                    blocked_reason: None,
                    dependencies: vec![],
                    results: None,
                    handoff: None,
                    legacy: false,
                },
                BacklogEntry {
                    item: Item {
                        id: "t-002".into(),
                        kind: KindMarker::new(),
                        claim: "Task 2".into(),
                        justifications: vec![],
                        status: BacklogStatus::Active,
                        supersedes: vec![],
                        superseded_by: None,
                        defeated_by: None,
                        authored_at: "2026-04-01T00:00:00Z".into(),
                        authored_in: "test".into(),
                    },
                    category: "x".into(),
                    blocked_reason: None,
                    dependencies: vec![],
                    results: None,
                    handoff: None,
                    legacy: false,
                },
            ],
        };
        write_backlog(plan, &backlog).unwrap();
    }

    #[test]
    fn apply_writes_intents_and_attributes_items() {
        let tmp = TempDir::new().unwrap();
        backlog_with_two_items(tmp.path());

        let proposal = IntentProposal {
            intents: vec![],
            item_attributions: vec![
                super::super::proposals::ItemAttribution {
                    item_id: "t-001".into(),
                    serves: "i-001".into(),
                },
                super::super::proposals::ItemAttribution {
                    item_id: "t-002".into(),
                    serves: "legacy".into(),
                },
            ],
        };
        std::fs::write(
            tmp.path().join(INTENT_PROPOSAL_FILENAME),
            serde_yaml::to_string(&proposal).unwrap(),
        )
        .unwrap();

        apply_proposal(tmp.path(), true).unwrap();

        let backlog = crate::state::backlog::yaml_io::read_backlog(tmp.path()).unwrap();
        let t1 = backlog.items.iter().find(|e| e.item.id == "t-001").unwrap();
        assert!(matches!(
            t1.item.justifications.last(),
            Some(Justification::ServesIntent { intent_id }) if intent_id == "i-001"
        ));
        assert!(!t1.legacy);

        let t2 = backlog.items.iter().find(|e| e.item.id == "t-002").unwrap();
        assert!(t2.legacy);
        assert!(t2.item.justifications.is_empty());

        assert!(
            !tmp.path().join(INTENT_PROPOSAL_FILENAME).exists(),
            "scratch removed"
        );
    }

    #[test]
    fn apply_errors_on_unknown_item_id() {
        let tmp = TempDir::new().unwrap();
        backlog_with_two_items(tmp.path());
        let proposal = IntentProposal {
            intents: vec![],
            item_attributions: vec![super::super::proposals::ItemAttribution {
                item_id: "t-nonexistent".into(),
                serves: "legacy".into(),
            }],
        };
        std::fs::write(
            tmp.path().join(INTENT_PROPOSAL_FILENAME),
            serde_yaml::to_string(&proposal).unwrap(),
        )
        .unwrap();

        let err = apply_proposal(tmp.path(), true).unwrap_err();
        assert!(format!("{err:#}").contains("unknown backlog item id"));
    }

    #[test]
    fn apply_errors_when_scratch_missing() {
        let tmp = TempDir::new().unwrap();
        backlog_with_two_items(tmp.path());
        let err = apply_proposal(tmp.path(), true).unwrap_err();
        assert!(format!("{err:#}").contains(INTENT_PROPOSAL_FILENAME));
    }
}
