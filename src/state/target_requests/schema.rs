//! Typed schema for `<plan>/target-requests.yaml`.
//!
//! `target-requests.yaml` is a transient scratch file: ravel-lite-create
//! seeds it from the user's initial component references, the work phase
//! may append new requests when the LLM realises it needs additional
//! mounted code, and the runner drains it at every phase boundary —
//! mounting each request via `mount_target` and deleting the file. See
//! `docs/architecture-next.md` §Dynamic mounting and §Phase boundaries.
//!
//! A `TargetRequest` is just a ComponentRef plus a free-form reason.
//! Unlike `intents.yaml`, `backlog.yaml`, and `memory.yaml`, requests
//! are NOT TMS-shaped knowledge items — they are a one-shot to-do list
//! the runner mechanically processes and removes.

use serde::{Deserialize, Serialize};

use crate::component_ref::ComponentRef;

pub const TARGET_REQUESTS_SCHEMA_VERSION: u32 = 1;

/// One mount request. The `component` field is a typed `ComponentRef`;
/// its serde impl uses the `<repo>:<component>` string form on the wire,
/// matching `commits.yaml`, `targets.yaml`, and the rest of the v2
/// surface (architecture-next §Dynamic mounting). The `reason` is
/// free-form text that lets a human reading the queue understand why a
/// mount was requested — useful when triaging a stale or
/// partially-drained file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetRequest {
    pub component: ComponentRef,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetRequestsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub requests: Vec<TargetRequest>,
}

impl Default for TargetRequestsFile {
    fn default() -> Self {
        Self {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request(repo: &str, component: &str, reason: &str) -> TargetRequest {
        TargetRequest {
            component: ComponentRef::new(repo, component),
            reason: reason.to_string(),
        }
    }

    #[test]
    fn target_request_component_serialises_as_string_scalar() {
        // Wire-shape compatibility: even though `component` is now a
        // typed `ComponentRef`, its serde impl emits a single string
        // scalar — so existing on-disk files (and existing prompts that
        // emit `component: <repo>:<id>`) continue to round-trip.
        let req = sample_request("atlas", "atlas-ontology", "core schema");
        let yaml = serde_yaml::to_string(&req).unwrap();
        assert!(
            yaml.contains("component: atlas:atlas-ontology"),
            "component must serialise as the `<repo>:<id>` string form: {yaml}"
        );
    }

    #[test]
    fn target_request_round_trips_through_yaml() {
        let req = sample_request("atlas", "atlas-ontology", "needs catalog crate");
        let yaml = serde_yaml::to_string(&req).unwrap();
        let decoded: TargetRequest = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, req);
    }

    #[test]
    fn target_requests_file_default_has_current_schema_version() {
        let file = TargetRequestsFile::default();
        assert_eq!(file.schema_version, TARGET_REQUESTS_SCHEMA_VERSION);
        assert!(file.requests.is_empty());
    }

    #[test]
    fn target_requests_file_round_trips_through_yaml() {
        let file = TargetRequestsFile {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: vec![
                sample_request("atlas", "atlas-ontology", "core schema"),
                sample_request("sidekick", "router", "edit handlers"),
            ],
        };
        let yaml = serde_yaml::to_string(&file).unwrap();
        let decoded: TargetRequestsFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, file);
    }

    #[test]
    fn target_requests_file_rejects_yaml_without_schema_version() {
        let yaml = "requests: []\n";
        let result: Result<TargetRequestsFile, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "schema_version is required; missing must fail"
        );
    }

    #[test]
    fn target_requests_file_accepts_yaml_without_requests_key() {
        let yaml = "schema_version: 1\n";
        let parsed: TargetRequestsFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.schema_version, TARGET_REQUESTS_SCHEMA_VERSION);
        assert!(parsed.requests.is_empty());
    }
}
