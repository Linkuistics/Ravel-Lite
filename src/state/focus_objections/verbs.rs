//! Handlers for every `state focus-objections <verb>` CLI verb.
//!
//! The objection vocabulary is closed and load-bearing — triage drives
//! mechanical hygiene off the typed `kind`, and a hallucinated kind
//! would silently break that chain. Following the precedent set by
//! `state discover-proposals add-proposal`, the CLI exposes a separate
//! verb per kind (`add-wrong-target`, `add-skip-item`, `add-premature`)
//! rather than one stringly-typed `--kind` flag, so a typo at the
//! prompt boundary is rejected by clap before any YAML lands on disk.
//!
//! `list` shows the queue; `clear` drains the file (the work-side
//! analog of triage's automatic next-cycle drain). No `remove` verb —
//! objections are an append-only queue per cycle, then dropped wholesale
//! at the next triage boundary.

use std::path::Path;

use anyhow::Result;

use crate::bail_with;
use crate::cli::{ErrorCode, OutputFormat};
use crate::component_ref::ComponentRef;

use super::schema::{FocusObjectionsFile, Objection};
use super::yaml_io::{
    delete_focus_objections, read_focus_objections, write_focus_objections,
};

pub fn run_list(plan_dir: &Path, format: OutputFormat) -> Result<()> {
    let file = read_focus_objections(plan_dir)?;
    emit(&file, format)
}

pub fn run_clear(plan_dir: &Path) -> Result<()> {
    delete_focus_objections(plan_dir)
}

pub fn run_add_wrong_target(
    plan_dir: &Path,
    suggested_target: ComponentRef,
    reasoning: &str,
) -> Result<()> {
    require_non_empty(reasoning, "--reasoning")?;
    append(
        plan_dir,
        Objection::WrongTarget {
            suggested_target,
            reasoning: ensure_trailing_newline(reasoning),
        },
    )
}

pub fn run_add_skip_item(plan_dir: &Path, item_id: &str, reasoning: &str) -> Result<()> {
    require_non_empty(item_id, "--item-id")?;
    require_non_empty(reasoning, "--reasoning")?;
    append(
        plan_dir,
        Objection::SkipItem {
            item_id: item_id.to_string(),
            reasoning: ensure_trailing_newline(reasoning),
        },
    )
}

pub fn run_add_premature(plan_dir: &Path, reasoning: &str) -> Result<()> {
    require_non_empty(reasoning, "--reasoning")?;
    append(
        plan_dir,
        Objection::Premature {
            reasoning: ensure_trailing_newline(reasoning),
        },
    )
}

fn append(plan_dir: &Path, objection: Objection) -> Result<()> {
    let mut file = read_focus_objections(plan_dir)?;
    file.objections.push(objection);
    write_focus_objections(plan_dir, &file)
}

fn require_non_empty(value: &str, flag_name: &str) -> Result<()> {
    if value.is_empty() {
        bail_with!(ErrorCode::InvalidInput, "{flag_name} must be non-empty");
    }
    Ok(())
}

fn emit(file: &FocusObjectionsFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(file)?,
        OutputFormat::Json => serde_json::to_string_pretty(file)? + "\n",
        OutputFormat::Markdown => {
            bail_with!(
                ErrorCode::InvalidInput,
                "`state focus-objections` does not support --format markdown; use yaml or json"
            )
        }
    };
    print!("{serialised}");
    Ok(())
}

fn ensure_trailing_newline(body: &str) -> String {
    if body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn run_add_wrong_target_appends_typed_objection() {
        let tmp = TempDir::new().unwrap();
        run_add_wrong_target(
            tmp.path(),
            ComponentRef::new("atlas", "atlas-ontology"),
            "Need ontology change first",
        )
        .unwrap();

        let file = read_focus_objections(tmp.path()).unwrap();
        assert_eq!(file.objections.len(), 1);
        match &file.objections[0] {
            Objection::WrongTarget {
                suggested_target,
                reasoning,
            } => {
                assert_eq!(suggested_target, &ComponentRef::new("atlas", "atlas-ontology"));
                assert_eq!(reasoning, "Need ontology change first\n");
            }
            other => panic!("expected WrongTarget, got {other:?}"),
        }
    }

    #[test]
    fn run_add_skip_item_appends_typed_objection() {
        let tmp = TempDir::new().unwrap();
        run_add_skip_item(tmp.path(), "t-007", "Blocked upstream").unwrap();

        let file = read_focus_objections(tmp.path()).unwrap();
        assert_eq!(file.objections.len(), 1);
        assert!(matches!(
            &file.objections[0],
            Objection::SkipItem { item_id, .. } if item_id == "t-007"
        ));
    }

    #[test]
    fn run_add_premature_appends_typed_objection() {
        let tmp = TempDir::new().unwrap();
        run_add_premature(tmp.path(), "Understand X first").unwrap();

        let file = read_focus_objections(tmp.path()).unwrap();
        assert_eq!(file.objections.len(), 1);
        assert!(matches!(&file.objections[0], Objection::Premature { .. }));
    }

    #[test]
    fn each_add_verb_rejects_empty_reasoning() {
        let tmp = TempDir::new().unwrap();
        assert!(run_add_wrong_target(tmp.path(), ComponentRef::new("a", "b"), "").is_err());
        assert!(run_add_skip_item(tmp.path(), "t-1", "").is_err());
        assert!(run_add_premature(tmp.path(), "").is_err());
    }

    #[test]
    fn run_add_skip_item_rejects_empty_item_id() {
        let tmp = TempDir::new().unwrap();
        let err = run_add_skip_item(tmp.path(), "", "reasoning").unwrap_err();
        assert!(format!("{err:#}").contains("--item-id"));
    }

    #[test]
    fn objections_accumulate_across_calls() {
        let tmp = TempDir::new().unwrap();
        run_add_wrong_target(tmp.path(), ComponentRef::new("a", "b"), "r1").unwrap();
        run_add_skip_item(tmp.path(), "t-1", "r2").unwrap();
        run_add_premature(tmp.path(), "r3").unwrap();

        let file = read_focus_objections(tmp.path()).unwrap();
        assert_eq!(file.objections.len(), 3);
        assert_eq!(file.objections[0].kind_str(), "wrong-target");
        assert_eq!(file.objections[1].kind_str(), "skip-item");
        assert_eq!(file.objections[2].kind_str(), "premature");
    }

    #[test]
    fn run_clear_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        run_clear(tmp.path()).unwrap();
        run_add_premature(tmp.path(), "r").unwrap();
        run_clear(tmp.path()).unwrap();
        run_clear(tmp.path()).unwrap();
        let file = read_focus_objections(tmp.path()).unwrap();
        assert!(file.objections.is_empty());
    }
}
