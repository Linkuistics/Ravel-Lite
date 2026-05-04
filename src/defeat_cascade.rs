//! Plan-state defeat cascade: propagate intent terminal-state changes
//! through `serves-intent` edges to the backlog.
//!
//! Architecture-next §Phase boundaries lists the cascade as one of the
//! mechanical things the runner does between phases. §TRIAGE step 2
//! pins the contract: "Mechanically propagate intent status changes
//! through `serves-intent` edges to backlog items. Run by the runner,
//! not the LLM."
//!
//! The actual cascade algorithm lives in the `knowledge_graph`
//! substrate as `cascade_serves_intent`. This module is the plan-side
//! disk wrapper: load `intents.yaml` and `backlog.yaml` into typed
//! stores, run the substrate cascade, mirror the resulting status /
//! `defeated_by` flips back into the on-disk `BacklogEntry` records
//! (so backlog-only extension fields like `category` and
//! `dependencies` survive the round-trip), and write the backlog back
//! atomically only when at least one item flipped.
//!
//! Wire-up note: this function is built but intentionally not yet
//! invoked from `phase_loop`. Live plans (notably the dogfood plan
//! `LLM_STATE/core`) still carry the v1 backlog wire shape, which
//! `read_backlog` rejects on `schema_version` mismatch. Wiring the
//! cascade into the phase loop must wait for the v1→v2 migrator to
//! ship — see the `LLM_STATE-shape-frozen` memory entry for the freeze
//! contract.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use knowledge_graph::cascade_serves_intent;

use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::plan_kg::{BacklogStatus, BacklogStore, IntentStore};
use crate::state::backlog::{read_backlog, write_backlog};
use crate::state::intents::read_intents;

/// Apply the serves-intent defeat cascade to the plan's backlog.
///
/// For every active backlog item whose `serves-intent` justifications
/// all point at terminal (defeated / satisfied / superseded) intents,
/// set the item's status to `Defeated` and `defeated_by: cascade`.
/// Returns the ids of items newly cascade-defeated by this call.
///
/// Disk write is conditional on at least one cascade hit: a no-op
/// cascade leaves `backlog.yaml` byte-identical so the surrounding
/// commit captures no spurious churn. Extension fields (`category`,
/// `dependencies`, `results`, `handoff`, `blocked_reason`) are
/// preserved by mirroring only the cascade-induced `status` /
/// `defeated_by` changes back into the existing entries rather than
/// reconstructing entries from the typed store.
pub fn run_defeat_cascade(plan_dir: &Path) -> Result<Vec<String>> {
    let intents_file = read_intents(plan_dir)
        .context("Defeat cascade: failed to read intents.yaml")
        .with_code(ErrorCode::IoError)?;
    let mut backlog_file = read_backlog(plan_dir)
        .context("Defeat cascade: failed to read backlog.yaml")
        .with_code(ErrorCode::IoError)?;

    let mut intent_store = IntentStore::new();
    for entry in &intents_file.items {
        intent_store
            .insert(entry.item.clone())
            .with_context(|| {
                format!(
                    "Defeat cascade: failed to materialise intent {} into store",
                    entry.item.id
                )
            })
            .with_code(ErrorCode::Internal)?;
    }

    let mut backlog_store = BacklogStore::new();
    for entry in &backlog_file.items {
        backlog_store
            .insert(entry.item.clone())
            .with_context(|| {
                format!(
                    "Defeat cascade: failed to materialise backlog item {} into store",
                    entry.item.id
                )
            })
            .with_code(ErrorCode::Internal)?;
    }

    let newly_defeated =
        cascade_serves_intent(&intent_store, &mut backlog_store, BacklogStatus::Defeated);

    if newly_defeated.is_empty() {
        return Ok(Vec::new());
    }

    let id_set: HashSet<&String> = newly_defeated.iter().collect();
    for entry in backlog_file.items.iter_mut() {
        if id_set.contains(&entry.item.id) {
            let updated = backlog_store
                .get(&entry.item.id)
                .expect("id drawn from cascade_serves_intent return value");
            entry.item.status = updated.status;
            entry.item.defeated_by = updated.defeated_by.clone();
        }
    }

    write_backlog(plan_dir, &backlog_file)
        .context("Defeat cascade: failed to write updated backlog.yaml")
        .with_code(ErrorCode::IoError)?;

    Ok(newly_defeated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_kg::{BacklogItemKind, BacklogStatus, IntentKind, IntentStatus};
    use crate::state::backlog::schema::{BacklogEntry, BacklogFile, BACKLOG_SCHEMA_VERSION};
    use crate::state::backlog::write_backlog;
    use crate::state::backlog::yaml_io::backlog_path;
    use crate::state::intents::schema::{IntentEntry, IntentsFile, INTENTS_SCHEMA_VERSION};
    use crate::state::intents::write_intents;
    use knowledge_graph::{DefeatedBy, Item, Justification, KindMarker};
    use tempfile::TempDir;

    fn intent(id: &str, status: IntentStatus) -> IntentEntry {
        IntentEntry {
            item: Item::<IntentKind> {
                id: id.into(),
                kind: KindMarker::new(),
                claim: format!("intent {id}"),
                justifications: vec![],
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-05-02T00:00:00Z".into(),
                authored_in: "test".into(),
            },
        }
    }

    fn backlog_item_serving(
        id: &str,
        intent_id: &str,
        status: BacklogStatus,
    ) -> BacklogEntry {
        BacklogEntry {
            item: Item::<BacklogItemKind> {
                id: id.into(),
                kind: KindMarker::new(),
                claim: format!("backlog {id}"),
                justifications: vec![Justification::ServesIntent {
                    intent_id: intent_id.into(),
                }],
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-05-02T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            category: "architecture-next".into(),
            blocked_reason: None,
            dependencies: vec![],
            results: None,
            handoff: None,
        }
    }

    fn write_pair(plan_dir: &Path, intents: Vec<IntentEntry>, backlog: Vec<BacklogEntry>) {
        write_intents(
            plan_dir,
            &IntentsFile {
                schema_version: INTENTS_SCHEMA_VERSION,
                items: intents,
            },
        )
        .unwrap();
        write_backlog(
            plan_dir,
            &BacklogFile {
                schema_version: BACKLOG_SCHEMA_VERSION,
                items: backlog,
            },
        )
        .unwrap();
    }

    #[test]
    fn cascade_flips_dependents_when_intent_is_defeated() {
        let tmp = TempDir::new().unwrap();
        write_pair(
            tmp.path(),
            vec![
                intent("i-1", IntentStatus::Defeated),
                intent("i-2", IntentStatus::Active),
            ],
            vec![
                backlog_item_serving("t-1", "i-1", BacklogStatus::Active),
                backlog_item_serving("t-2", "i-2", BacklogStatus::Active),
            ],
        );

        let cascaded = run_defeat_cascade(tmp.path()).unwrap();

        assert_eq!(cascaded, vec!["t-1".to_string()]);
        let on_disk = read_backlog(tmp.path()).unwrap();
        let t1 = on_disk.items.iter().find(|e| e.item.id == "t-1").unwrap();
        let t2 = on_disk.items.iter().find(|e| e.item.id == "t-2").unwrap();
        assert_eq!(t1.item.status, BacklogStatus::Defeated);
        assert_eq!(t1.item.defeated_by, Some(DefeatedBy::Cascade));
        assert_eq!(t2.item.status, BacklogStatus::Active);
        assert_eq!(t2.item.defeated_by, None);
    }

    #[test]
    fn cascade_is_no_op_when_no_intent_is_terminal() {
        let tmp = TempDir::new().unwrap();
        write_pair(
            tmp.path(),
            vec![intent("i-1", IntentStatus::Active)],
            vec![backlog_item_serving("t-1", "i-1", BacklogStatus::Active)],
        );
        let before = std::fs::read(backlog_path(tmp.path())).unwrap();

        let cascaded = run_defeat_cascade(tmp.path()).unwrap();

        assert!(cascaded.is_empty());
        let after = std::fs::read(backlog_path(tmp.path())).unwrap();
        assert_eq!(
            before, after,
            "no-op cascade must leave backlog.yaml byte-identical"
        );
    }

    #[test]
    fn cascade_treats_satisfied_intent_as_terminal() {
        // `is_terminal` for IntentStatus is `!matches!(self, Active)`,
        // so Satisfied also defeats dependents. Guards against a
        // future regression where someone narrows the cascade to
        // `Defeated`-only and quietly breaks the satisfied case.
        let tmp = TempDir::new().unwrap();
        write_pair(
            tmp.path(),
            vec![intent("i-1", IntentStatus::Satisfied)],
            vec![backlog_item_serving("t-1", "i-1", BacklogStatus::Active)],
        );

        let cascaded = run_defeat_cascade(tmp.path()).unwrap();

        assert_eq!(cascaded, vec!["t-1".to_string()]);
    }

    #[test]
    fn cascade_preserves_extension_fields_on_flipped_items() {
        let tmp = TempDir::new().unwrap();
        let mut entry = backlog_item_serving("t-1", "i-1", BacklogStatus::Active);
        entry.dependencies = vec!["t-0".into()];
        entry.results = Some("partial work landed earlier.\n".into());
        write_pair(
            tmp.path(),
            vec![intent("i-1", IntentStatus::Defeated)],
            vec![entry],
        );

        run_defeat_cascade(tmp.path()).unwrap();

        let on_disk = read_backlog(tmp.path()).unwrap();
        let t1 = &on_disk.items[0];
        assert_eq!(t1.category, "architecture-next");
        assert_eq!(t1.dependencies, vec!["t-0".to_string()]);
        assert_eq!(t1.results.as_deref(), Some("partial work landed earlier.\n"));
    }

    #[test]
    fn cascade_skips_items_whose_serves_intent_targets_active_intents() {
        // Item serving two intents: one defeated, one active. The
        // substrate spec is "all serves-intent terminal" → don't
        // cascade. Pin the spec at the wrapper level too.
        let tmp = TempDir::new().unwrap();
        let mut entry = backlog_item_serving("t-1", "i-1", BacklogStatus::Active);
        entry.item.justifications.push(Justification::ServesIntent {
            intent_id: "i-2".into(),
        });
        write_pair(
            tmp.path(),
            vec![
                intent("i-1", IntentStatus::Defeated),
                intent("i-2", IntentStatus::Active),
            ],
            vec![entry],
        );

        let cascaded = run_defeat_cascade(tmp.path()).unwrap();

        assert!(cascaded.is_empty());
        let on_disk = read_backlog(tmp.path()).unwrap();
        assert_eq!(on_disk.items[0].item.status, BacklogStatus::Active);
    }

    #[test]
    fn cascade_errors_when_intents_yaml_is_missing() {
        // Wire-up gate: callers must not invoke run_defeat_cascade on
        // plans that haven't migrated to the v2 wire shape. A clear
        // error here is what makes "deploy gated on migrator" safe.
        let tmp = TempDir::new().unwrap();
        write_backlog(
            tmp.path(),
            &BacklogFile {
                schema_version: BACKLOG_SCHEMA_VERSION,
                items: vec![],
            },
        )
        .unwrap();
        let err = run_defeat_cascade(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("intents.yaml"),
            "error must cite intents.yaml: {msg}"
        );
    }

    #[test]
    fn cascade_errors_when_backlog_yaml_is_missing() {
        let tmp = TempDir::new().unwrap();
        write_intents(
            tmp.path(),
            &IntentsFile {
                schema_version: INTENTS_SCHEMA_VERSION,
                items: vec![],
            },
        )
        .unwrap();
        let err = run_defeat_cascade(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("backlog.yaml"),
            "error must cite backlog.yaml: {msg}"
        );
    }
}
