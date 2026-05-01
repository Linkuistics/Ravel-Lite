//! Handlers for every `state target-requests <verb>` CLI verb.
//!
//! Requests are scratch entries, not TMS items, so the verb surface
//! mirrors `state targets` rather than the intents/memory/backlog CRUD
//! verbs:
//!
//! - No `set-status`/`set-body`: a request either exists in the queue
//!   or it doesn't.
//! - Identity is the `<repo>:<component>` reference (the same notation
//!   `state targets` uses), addressed via a single positional
//!   argument.
//!
//! See `docs/architecture-next.md` §Dynamic mounting for the contract.

use std::path::Path;

use anyhow::{bail, Result};

use super::schema::{TargetRequest, TargetRequestsFile, TARGET_REQUESTS_SCHEMA_VERSION};
use super::yaml_io::{read_target_requests, write_target_requests};

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
    let requests = read_target_requests(plan_dir)?;
    emit(&requests, format)
}

pub fn run_show(plan_dir: &Path, reference: &str, format: OutputFormat) -> Result<()> {
    validate_reference(reference)?;
    let requests = read_target_requests(plan_dir)?;
    let entry = find_request(&requests, reference)?;
    let wrapper = TargetRequestsFile {
        schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
        requests: vec![entry.clone()],
    };
    emit(&wrapper, format)
}

pub fn run_add(plan_dir: &Path, reference: &str, reason: &str) -> Result<()> {
    validate_reference(reference)?;
    if reason.is_empty() {
        bail!("--reason must be non-empty");
    }
    let mut file = read_target_requests(plan_dir)?;
    if find_request(&file, reference).is_ok() {
        bail!("request for {reference} already queued");
    }
    file.requests.push(TargetRequest {
        component: reference.to_string(),
        reason: reason.to_string(),
    });
    write_target_requests(plan_dir, &file)
}

pub fn run_remove(plan_dir: &Path, reference: &str) -> Result<()> {
    validate_reference(reference)?;
    let mut file = read_target_requests(plan_dir)?;
    let before = file.requests.len();
    file.requests.retain(|r| r.component != reference);
    if file.requests.len() == before {
        bail!("no request for {reference} to remove");
    }
    write_target_requests(plan_dir, &file)
}

/// Reject malformed `<repo>:<component>` references at the boundary so
/// no badly shaped row ever lands on disk. Mirrors
/// `state::targets::verbs::parse_reference` semantics — both halves
/// non-empty, exactly one colon — without allocating the split halves
/// (we keep the original string for storage).
pub(crate) fn validate_reference(reference: &str) -> Result<()> {
    match reference.split_once(':') {
        Some((repo, component)) if !repo.is_empty() && !component.is_empty() => Ok(()),
        _ => bail!(
            "target reference {reference:?} must be `<repo_slug>:<component_id>` with both parts non-empty"
        ),
    }
}

pub(crate) fn find_request<'a>(
    file: &'a TargetRequestsFile,
    reference: &str,
) -> Result<&'a TargetRequest> {
    file.requests
        .iter()
        .find(|r| r.component == reference)
        .ok_or_else(|| anyhow::anyhow!("no request for {reference}"))
}

fn emit(file: &TargetRequestsFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(file)?,
        OutputFormat::Json => serde_json::to_string_pretty(file)? + "\n",
    };
    print!("{serialised}");
    Ok(())
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
    fn run_add_appends_request_to_empty_file() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), "atlas:atlas-ontology", "core schema").unwrap();

        let updated = read_target_requests(tmp.path()).unwrap();
        assert_eq!(updated.requests.len(), 1);
        assert_eq!(updated.requests[0].component, "atlas:atlas-ontology");
        assert_eq!(updated.requests[0].reason, "core schema");
    }

    #[test]
    fn run_add_rejects_duplicate_reference() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), "atlas:atlas-ontology", "first").unwrap();
        let err = run_add(tmp.path(), "atlas:atlas-ontology", "second").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("already queued"), "error must say already queued: {msg}");
    }

    #[test]
    fn run_add_rejects_empty_reason() {
        let tmp = TempDir::new().unwrap();
        let err = run_add(tmp.path(), "atlas:atlas-ontology", "").unwrap_err();
        assert!(format!("{err:#}").contains("--reason"));
    }

    #[test]
    fn run_add_rejects_malformed_reference() {
        let tmp = TempDir::new().unwrap();
        let err = run_add(tmp.path(), "atlas-only", "reason").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("repo_slug"), "error must explain expected shape: {msg}");
    }

    #[test]
    fn run_remove_drops_only_the_named_request() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), "atlas:ontology", "first").unwrap();
        run_add(tmp.path(), "sidekick:router", "second").unwrap();

        run_remove(tmp.path(), "atlas:ontology").unwrap();

        let updated = read_target_requests(tmp.path()).unwrap();
        assert_eq!(updated.requests.len(), 1);
        assert_eq!(updated.requests[0].component, "sidekick:router");
    }

    #[test]
    fn run_remove_errors_when_reference_not_present() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), "atlas:ontology", "reason").unwrap();
        let err = run_remove(tmp.path(), "missing:thing").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("missing:thing"), "error must cite the bad ref: {msg}");
    }

    #[test]
    fn find_request_returns_match() {
        let file = TargetRequestsFile {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: vec![TargetRequest {
                component: "atlas:ontology".into(),
                reason: "x".into(),
            }],
        };
        let found = find_request(&file, "atlas:ontology").unwrap();
        assert_eq!(found.component, "atlas:ontology");
    }
}
