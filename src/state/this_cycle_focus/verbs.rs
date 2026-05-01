//! Handlers for every `state this-cycle-focus <verb>` CLI verb.
//!
//! The focus record is single-document (not a list), so the verb surface
//! is `set` / `show` / `clear` rather than the list/add/remove shape
//! used by `state target-requests`.
//!
//! - `set` writes the whole document, replacing any prior content.
//! - `show` emits the document or errors when absent.
//! - `clear` removes the file (idempotent).
//!
//! Backlog item ids are validated as non-empty strings; the verb does
//! NOT cross-check them against `backlog.yaml` because triage may
//! emit ids for items it is about to add. Cross-validation, if ever
//! wanted, belongs in a triage-time linter rather than at the CRUD
//! boundary.

use std::path::Path;

use anyhow::{bail, Result};

use super::schema::{ThisCycleFocus, THIS_CYCLE_FOCUS_SCHEMA_VERSION};
use super::yaml_io::{delete_this_cycle_focus, read_this_cycle_focus, write_this_cycle_focus};

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

pub fn run_show(plan_dir: &Path, format: OutputFormat) -> Result<()> {
    let focus = read_this_cycle_focus(plan_dir)?
        .ok_or_else(|| anyhow::anyhow!("no this-cycle focus is set"))?;
    emit(&focus, format)
}

pub fn run_set(
    plan_dir: &Path,
    target: &str,
    backlog_items: &[String],
    notes: Option<&str>,
) -> Result<()> {
    validate_reference(target)?;
    for id in backlog_items {
        if id.is_empty() {
            bail!("--item ids must be non-empty");
        }
    }
    let focus = ThisCycleFocus {
        schema_version: THIS_CYCLE_FOCUS_SCHEMA_VERSION,
        target: target.to_string(),
        backlog_items: backlog_items.to_vec(),
        notes: notes.map(ensure_trailing_newline),
    };
    write_this_cycle_focus(plan_dir, &focus)
}

pub fn run_clear(plan_dir: &Path) -> Result<()> {
    delete_this_cycle_focus(plan_dir)
}

/// Reject malformed `<repo>:<component>` references at the CLI boundary.
/// Mirrors `state::target_requests::verbs::validate_reference` so the
/// reference grammar stays uniform across every v2 scratch file.
pub(crate) fn validate_reference(reference: &str) -> Result<()> {
    match reference.split_once(':') {
        Some((repo, component)) if !repo.is_empty() && !component.is_empty() => Ok(()),
        _ => bail!(
            "target reference {reference:?} must be `<repo_slug>:<component_id>` with both parts non-empty"
        ),
    }
}

fn emit(focus: &ThisCycleFocus, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(focus)?,
        OutputFormat::Json => serde_json::to_string_pretty(focus)? + "\n",
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
    fn validate_reference_accepts_well_formed() {
        validate_reference("atlas:atlas-ontology").unwrap();
    }

    #[test]
    fn validate_reference_rejects_missing_colon() {
        let err = validate_reference("atlas-only").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("repo_slug"), "error must explain expected shape: {msg}");
    }

    #[test]
    fn validate_reference_rejects_empty_repo_or_component() {
        assert!(validate_reference(":only-component").is_err());
        assert!(validate_reference("only-repo:").is_err());
    }

    #[test]
    fn run_set_writes_focus_with_all_fields() {
        let tmp = TempDir::new().unwrap();
        run_set(
            tmp.path(),
            "atlas:atlas-core",
            &["t-001".into(), "t-005".into()],
            Some("Order: t-001 then t-005."),
        )
        .unwrap();

        let focus = read_this_cycle_focus(tmp.path()).unwrap().unwrap();
        assert_eq!(focus.target, "atlas:atlas-core");
        assert_eq!(focus.backlog_items, vec!["t-001", "t-005"]);
        assert_eq!(
            focus.notes.as_deref(),
            Some("Order: t-001 then t-005.\n"),
            "notes should have trailing newline normalised"
        );
    }

    #[test]
    fn run_set_overwrites_prior_focus() {
        let tmp = TempDir::new().unwrap();
        run_set(tmp.path(), "atlas:atlas-core", &["t-001".into()], None).unwrap();
        run_set(tmp.path(), "sidekick:router", &[], Some("Different focus.")).unwrap();

        let focus = read_this_cycle_focus(tmp.path()).unwrap().unwrap();
        assert_eq!(focus.target, "sidekick:router");
        assert!(focus.backlog_items.is_empty());
        assert_eq!(focus.notes.as_deref(), Some("Different focus.\n"));
    }

    #[test]
    fn run_set_rejects_malformed_target() {
        let tmp = TempDir::new().unwrap();
        let err = run_set(tmp.path(), "atlas-only", &[], None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("repo_slug"), "error must explain expected shape: {msg}");
    }

    #[test]
    fn run_set_rejects_empty_item_id() {
        let tmp = TempDir::new().unwrap();
        let err = run_set(tmp.path(), "atlas:core", &["".into()], None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("non-empty"), "error must mention non-empty: {msg}");
    }

    #[test]
    fn run_clear_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        run_clear(tmp.path()).unwrap();
        run_set(tmp.path(), "atlas:core", &[], None).unwrap();
        run_clear(tmp.path()).unwrap();
        run_clear(tmp.path()).unwrap();
        assert!(read_this_cycle_focus(tmp.path()).unwrap().is_none());
    }
}
