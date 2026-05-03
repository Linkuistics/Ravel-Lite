//! `ravel-lite capabilities` — machine-readable summary of what this
//! binary can do (cli-tool-design.md §"Optional but valuable").
//!
//! Lets an agent probe feature support without parsing `--help` output:
//! version, top-level subcommands, supported output formats, the
//! stable error-code vocabulary, the documented exit-category table,
//! and a feature-flags object. JSON only — agents are the audience.
//!
//! The schema is versioned via the top-level `schema_version` field;
//! bump on incompatible changes (renaming/removing fields). Adding new
//! fields is non-breaking.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::cli::error_code::ErrorCode;
use crate::cli::exit_category::ExitCategory;

/// Bump on incompatible field changes. Adding fields is non-breaking.
pub const CAPABILITIES_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub schema_version: u32,
    pub version: &'static str,
    pub subcommands: Vec<&'static str>,
    pub output_formats: Vec<&'static str>,
    pub error_codes: Vec<&'static str>,
    pub exit_categories: Vec<ExitCategoryRecord>,
    pub features: FeatureFlags,
}

#[derive(Debug, Serialize)]
pub struct ExitCategoryRecord {
    pub code: i32,
    pub label: &'static str,
}

#[derive(Debug, Serialize)]
pub struct FeatureFlags {
    pub json_output: bool,
    pub yaml_output: bool,
    pub markdown_output: bool,
    pub stable_error_codes: bool,
    pub documented_exit_codes: bool,
    pub schema_versioning: bool,
}

/// Build the capabilities document with the current binary's view of
/// what it supports. `version_string` is the same value rendered by
/// `ravel-lite version` / `--version`.
pub fn build(version_string: &'static str) -> Capabilities {
    Capabilities {
        schema_version: CAPABILITIES_SCHEMA_VERSION,
        version: version_string,
        subcommands: top_level_subcommands(),
        output_formats: vec!["yaml", "json", "markdown"],
        error_codes: ErrorCode::all().iter().map(ErrorCode::as_str).collect(),
        exit_categories: ExitCategory::documented()
            .iter()
            .map(|cat| ExitCategoryRecord {
                code: cat.as_code(),
                label: cat.label(),
            })
            .collect(),
        features: FeatureFlags {
            json_output: true,
            yaml_output: true,
            markdown_output: true,
            stable_error_codes: true,
            documented_exit_codes: true,
            schema_versioning: true,
        },
    }
}

/// Stable list of top-level subcommands the binary exposes. Kept in
/// sync with `Commands` in `src/main.rs` by hand — there are too few
/// to justify a derive.
fn top_level_subcommands() -> Vec<&'static str> {
    vec![
        "init",
        "run",
        "create",
        "survey",
        "survey-format",
        "version",
        "capabilities",
        "state",
        "repo",
        "plan",
        "findings",
        "atlas",
        "fixed-memory",
    ]
}

/// CLI handler: emit the capabilities document as pretty JSON with a
/// trailing newline.
pub fn run(version_string: &'static str) -> Result<()> {
    let caps = build(version_string);
    let json = serde_json::to_string_pretty(&caps)
        .context("Failed to serialise capabilities document to JSON")?;
    println!("{json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_carries_schema_version() {
        let caps = build("test");
        assert_eq!(caps.schema_version, CAPABILITIES_SCHEMA_VERSION);
    }

    #[test]
    fn every_error_code_appears_in_capabilities() {
        let caps = build("test");
        let listed: std::collections::HashSet<&str> = caps.error_codes.iter().copied().collect();
        for code in ErrorCode::all() {
            assert!(
                listed.contains(code.as_str()),
                "ErrorCode::{code:?} missing from capabilities.error_codes"
            );
        }
    }

    #[test]
    fn every_exit_category_appears_in_capabilities() {
        let caps = build("test");
        let codes: std::collections::HashSet<i32> =
            caps.exit_categories.iter().map(|c| c.code).collect();
        for cat in ExitCategory::documented() {
            assert!(
                codes.contains(&cat.as_code()),
                "ExitCategory::{cat:?} missing from capabilities.exit_categories"
            );
        }
    }

    #[test]
    fn capabilities_serialises_as_well_formed_json() {
        let caps = build("0.1.0 (test)");
        let json = serde_json::to_string_pretty(&caps).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["schema_version"], 1);
        assert!(parsed["features"]["json_output"].as_bool().unwrap_or(false));
        assert!(parsed["subcommands"].as_array().unwrap().contains(&serde_json::json!("state")));
        assert!(parsed["output_formats"].as_array().unwrap().contains(&serde_json::json!("json")));
    }
}
