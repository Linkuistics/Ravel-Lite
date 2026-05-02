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
use crate::component_ref::ComponentRef;

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
    let reference: ComponentRef = reference.parse()?;
    let requests = read_target_requests(plan_dir)?;
    let entry = find_request(&requests, &reference)?;
    let wrapper = TargetRequestsFile {
        schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
        requests: vec![entry.clone()],
    };
    emit(&wrapper, format)
}

pub fn run_add(plan_dir: &Path, reference: &str, reason: &str) -> Result<()> {
    let reference: ComponentRef = reference.parse()?;
    if reason.is_empty() {
        bail!("--reason must be non-empty");
    }
    let mut file = read_target_requests(plan_dir)?;
    if find_request(&file, &reference).is_ok() {
        bail!("request for {reference} already queued");
    }
    file.requests.push(TargetRequest {
        component: reference,
        reason: reason.to_string(),
    });
    write_target_requests(plan_dir, &file)
}

pub fn run_remove(plan_dir: &Path, reference: &str) -> Result<()> {
    let reference: ComponentRef = reference.parse()?;
    let mut file = read_target_requests(plan_dir)?;
    let before = file.requests.len();
    file.requests.retain(|r| r.component != reference);
    if file.requests.len() == before {
        bail!("no request for {reference} to remove");
    }
    write_target_requests(plan_dir, &file)
}

pub(crate) fn find_request<'a>(
    file: &'a TargetRequestsFile,
    reference: &ComponentRef,
) -> Result<&'a TargetRequest> {
    file.requests
        .iter()
        .find(|r| &r.component == reference)
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
    fn run_add_appends_request_to_empty_file() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), "atlas:atlas-ontology", "core schema").unwrap();

        let updated = read_target_requests(tmp.path()).unwrap();
        assert_eq!(updated.requests.len(), 1);
        assert_eq!(
            updated.requests[0].component,
            ComponentRef::new("atlas", "atlas-ontology")
        );
        assert_eq!(updated.requests[0].reason, "core schema");
    }

    #[test]
    fn run_add_rejects_reference_with_empty_repo_slug() {
        let tmp = TempDir::new().unwrap();
        let err = run_add(tmp.path(), ":only-component", "reason").unwrap_err();
        assert!(format!("{err:#}").contains("repo_slug"));
    }

    #[test]
    fn run_add_rejects_reference_with_empty_component_id() {
        let tmp = TempDir::new().unwrap();
        let err = run_add(tmp.path(), "only-repo:", "reason").unwrap_err();
        assert!(format!("{err:#}").contains("component_id"));
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
        assert_eq!(
            updated.requests[0].component,
            ComponentRef::new("sidekick", "router")
        );
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
        let component = ComponentRef::new("atlas", "ontology");
        let file = TargetRequestsFile {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: vec![TargetRequest {
                component: component.clone(),
                reason: "x".into(),
            }],
        };
        let found = find_request(&file, &component).unwrap();
        assert_eq!(found.component, component);
    }
}
