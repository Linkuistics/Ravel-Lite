//! Handlers for every `state commits <verb>` CLI verb.
//!
//! `commits.yaml` is a one-shot scratch file (analyse-work writes it,
//! `git-commit-work` consumes it), so the verb surface is read-only:
//!
//! - `list` emits the whole file. Missing file is rendered as the empty
//!   default — same shape as `state target-requests list`.
//! - `show` emits a single entry by 1-based index. Commit specs have no
//!   stable identity field (no `id`, the message is free-form), so
//!   positional addressing is the most predictable mode.
//!
//! No `set` / `add` / `remove` verbs: the file is authored by the
//! analyse-work LLM phase, not by hand at the CLI.

use std::path::Path;

use anyhow::{bail, Result};

use super::schema::{CommitsSpec, COMMITS_SCHEMA_VERSION};
use super::yaml_io::read_commits;

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
    let spec = read_commits(plan_dir)?;
    emit(&spec, format)
}

pub fn run_show(plan_dir: &Path, index: usize, format: OutputFormat) -> Result<()> {
    if index == 0 {
        bail!("commit index must be 1-based; got 0");
    }
    let spec = read_commits(plan_dir)?;
    let entry = spec.commits.get(index - 1).ok_or_else(|| {
        anyhow::anyhow!(
            "no commit at index {index}; file holds {} entr{}",
            spec.commits.len(),
            if spec.commits.len() == 1 { "y" } else { "ies" }
        )
    })?;
    let wrapper = CommitsSpec {
        schema_version: COMMITS_SCHEMA_VERSION,
        commits: vec![entry.clone()],
    };
    emit(&wrapper, format)
}

fn emit(spec: &CommitsSpec, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(spec)?,
        OutputFormat::Json => serde_json::to_string_pretty(spec)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component_ref::ComponentRef;
    use crate::state::commits::schema::CommitSpec;
    use crate::state::commits::yaml_io::write_commits;
    use tempfile::TempDir;

    fn populated_spec() -> CommitsSpec {
        CommitsSpec {
            schema_version: COMMITS_SCHEMA_VERSION,
            commits: vec![
                CommitSpec {
                    paths: vec!["src/**".into()],
                    message: "first".into(),
                    target: Some(ComponentRef::new("ravel-lite", "phase-loop")),
                },
                CommitSpec {
                    paths: vec!["docs/**".into()],
                    message: "second".into(),
                    target: None,
                },
            ],
        }
    }

    #[test]
    fn run_list_emits_empty_default_when_file_missing() {
        // Smoke test: must not panic or error on a fresh plan dir.
        let tmp = TempDir::new().unwrap();
        run_list(tmp.path(), OutputFormat::Yaml).unwrap();
    }

    #[test]
    fn run_show_rejects_zero_index() {
        let tmp = TempDir::new().unwrap();
        write_commits(tmp.path(), &populated_spec()).unwrap();
        let err = run_show(tmp.path(), 0, OutputFormat::Yaml).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("1-based"),
            "error must explain indexing convention: {msg}"
        );
    }

    #[test]
    fn run_show_rejects_out_of_range_index() {
        let tmp = TempDir::new().unwrap();
        write_commits(tmp.path(), &populated_spec()).unwrap();
        let err = run_show(tmp.path(), 99, OutputFormat::Yaml).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no commit at index 99"));
        assert!(msg.contains("2 entries"), "error must show file size: {msg}");
    }

    #[test]
    fn run_show_succeeds_within_range() {
        let tmp = TempDir::new().unwrap();
        write_commits(tmp.path(), &populated_spec()).unwrap();
        run_show(tmp.path(), 1, OutputFormat::Yaml).unwrap();
        run_show(tmp.path(), 2, OutputFormat::Json).unwrap();
    }

    #[test]
    fn output_format_parses_known_values() {
        assert_eq!(OutputFormat::parse("yaml"), Some(OutputFormat::Yaml));
        assert_eq!(OutputFormat::parse("json"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::parse("toml"), None);
    }
}
