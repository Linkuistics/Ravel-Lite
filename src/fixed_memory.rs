//! Discovery and composition for the layered fixed-memory overlay.
//!
//! The "fixed memory" namespace holds reference material the LLM consults
//! during phase prompts (coding-style guides, memory-style rules, the
//! cli-tool-design checklist). Each entry is identified by a bare slug
//! that pins both an embedded shipped file and an optional per-user
//! override at `<config-dir>/fixed-memory/<slug>.md`.
//!
//! `discover` enumerates every slug across the two layers so `list` can
//! surface user additions to the LLM. `compose` resolves a single slug
//! for `show`, layering user content underneath the embedded content
//! with an addendum delimiter that signals precedence.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::init::{embedded_content, embedded_entries_with_prefix};

const FIXED_MEMORY_DIR: &str = "fixed-memory";
const EMBEDDED_PREFIX: &str = "fixed-memory/";
const SLUG_EXTENSION: &str = "md";

/// Delimiter inserted between the embedded body and the user addendum
/// when both layers exist. The H2 heading flags the user's content as
/// taking precedence over the embedded prose; the leading newline
/// guarantees separation even if the embedded file lacks a trailing
/// newline.
pub const ADDENDUM_DELIMITER: &str =
    "\n---\n\n## User addendum (takes precedence over the above)\n\n";

const LIST_SCHEMA_VERSION: u32 = 1;

/// Per-slug discovery record: which of the two layers contributed an
/// entry. At least one of `embedded` or `user_path` is `Some` for any
/// slug `discover` returns.
#[derive(Debug, Clone)]
pub struct EntrySources {
    pub embedded: Option<&'static str>,
    pub user_path: Option<PathBuf>,
}

/// `--format` values for `fixed-memory list`. Yaml is the default to
/// match existing `state <kind> list` verbs; markdown produces the table
/// form documented in the spec; json mirrors the yaml shape.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OutputFormat {
    Yaml,
    Json,
    Markdown,
}

impl OutputFormat {
    pub fn parse(input: &str) -> Option<OutputFormat> {
        match input {
            "yaml" => Some(OutputFormat::Yaml),
            "json" => Some(OutputFormat::Json),
            "markdown" => Some(OutputFormat::Markdown),
            _ => None,
        }
    }
}

/// Failure modes for `compose`. `UnknownSlug` carries the available slug
/// list so the CLI handler can surface it as remediation; `Io` covers
/// read failures against the user overlay path.
#[derive(Debug)]
pub enum ShowError {
    UnknownSlug { slug: String, available: Vec<String> },
    Io(io::Error),
}

impl std::fmt::Display for ShowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShowError::UnknownSlug { slug, available } => {
                let names = if available.is_empty() {
                    "(none)".to_string()
                } else {
                    available.join(", ")
                };
                write!(
                    f,
                    "no fixed-memory entry for slug {slug:?}. Available slugs: {names}. \
                     Run 'ravel-lite fixed-memory list' to inspect."
                )
            }
            ShowError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ShowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ShowError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for ShowError {
    fn from(err: io::Error) -> Self {
        ShowError::Io(err)
    }
}

/// Enumerate every fixed-memory slug across the embedded set and the
/// `<config-dir>/fixed-memory/` overlay. A missing or empty user dir is
/// not an error; only direct-child `*.md` files in the user dir become
/// entries.
pub fn discover(config_dir: &Path) -> Result<BTreeMap<String, EntrySources>> {
    let mut map: BTreeMap<String, EntrySources> = BTreeMap::new();

    for (path, body) in embedded_entries_with_prefix(EMBEDDED_PREFIX) {
        let leaf = path.strip_prefix(EMBEDDED_PREFIX).unwrap_or(path);
        if let Some(slug) = slug_from_filename(leaf) {
            map.entry(slug)
                .or_insert_with(EntrySources::empty)
                .embedded = Some(body);
        }
    }

    let user_dir = config_dir.join(FIXED_MEMORY_DIR);
    if user_dir.is_dir() {
        let read = fs::read_dir(&user_dir)
            .with_context(|| format!("Failed to read {}", user_dir.display()))?;
        for entry in read {
            let entry = entry.with_context(|| {
                format!("Failed to enumerate entries under {}", user_dir.display())
            })?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let leaf = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name,
                None => continue,
            };
            let Some(slug) = slug_from_filename(leaf) else {
                continue;
            };
            map.entry(slug)
                .or_insert_with(EntrySources::empty)
                .user_path = Some(path);
        }
    }

    Ok(map)
}

/// Resolve a single slug into its layered body. Embedded-only and
/// user-only cases emit content unchanged; the `both` case interposes
/// `ADDENDUM_DELIMITER` between the embedded body and the user body so
/// the LLM can see which guidance is the user's override.
pub fn compose(slug: &str, config_dir: &Path) -> Result<String, ShowError> {
    let embedded = embedded_content(&format!("{EMBEDDED_PREFIX}{slug}.{SLUG_EXTENSION}"));
    let user_path = config_dir
        .join(FIXED_MEMORY_DIR)
        .join(format!("{slug}.{SLUG_EXTENSION}"));
    let user_content = if user_path.is_file() {
        Some(fs::read_to_string(&user_path)?)
    } else {
        None
    };

    match (embedded, user_content) {
        (Some(body), None) => Ok(body.to_string()),
        (None, Some(body)) => Ok(body),
        (Some(emb), Some(usr)) => {
            let mut combined =
                String::with_capacity(emb.len() + ADDENDUM_DELIMITER.len() + usr.len());
            combined.push_str(emb);
            combined.push_str(ADDENDUM_DELIMITER);
            combined.push_str(&usr);
            Ok(combined)
        }
        (None, None) => Err(ShowError::UnknownSlug {
            slug: slug.to_string(),
            available: discover(config_dir)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default(),
        }),
    }
}

/// Extract the first H1 heading text from a markdown body. Leading
/// blank lines are skipped; the first non-blank line must be an H1
/// (`# ...`) or `None` is returned.
pub fn extract_description(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        return trimmed
            .strip_prefix("# ")
            .map(|rest| rest.trim().to_string());
    }
    None
}

/// Render the discovery map in the requested format. The `description`
/// is the embedded H1 when present, falling back to the user H1; entries
/// with neither omit the field rather than emit `null`.
pub fn render_list(
    map: &BTreeMap<String, EntrySources>,
    format: OutputFormat,
) -> Result<String> {
    let entries: Vec<ListedEntry> =
        map.iter().map(|(slug, sources)| project(slug, sources)).collect();
    let envelope = ListEnvelope {
        schema_version: LIST_SCHEMA_VERSION,
        entries,
    };

    Ok(match format {
        OutputFormat::Yaml => serde_yaml::to_string(&envelope)?,
        OutputFormat::Json => serde_json::to_string_pretty(&envelope)? + "\n",
        OutputFormat::Markdown => render_markdown(&envelope),
    })
}

fn project(slug: &str, sources: &EntrySources) -> ListedEntry {
    let description = sources.embedded.and_then(extract_description).or_else(|| {
        sources
            .user_path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| extract_description(&s))
    });
    let mut srcs: Vec<&'static str> = Vec::new();
    if sources.embedded.is_some() {
        srcs.push("embedded");
    }
    if sources.user_path.is_some() {
        srcs.push("user");
    }
    ListedEntry {
        slug: slug.to_string(),
        description,
        sources: srcs,
    }
}

fn render_markdown(envelope: &ListEnvelope) -> String {
    let mut out = String::new();
    out.push_str("| slug | description | sources |\n");
    out.push_str("|---|---|---|\n");
    for entry in &envelope.entries {
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            entry.slug,
            entry.description.as_deref().unwrap_or(""),
            entry.sources.join(", ")
        ));
    }
    out
}

fn slug_from_filename(name: &str) -> Option<String> {
    let stripped = name.strip_suffix(&format!(".{SLUG_EXTENSION}"))?;
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

impl EntrySources {
    fn empty() -> Self {
        EntrySources {
            embedded: None,
            user_path: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct ListedEntry {
    slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    sources: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct ListEnvelope {
    schema_version: u32,
    entries: Vec<ListedEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn extract_description_pulls_h1_text() {
        assert_eq!(
            extract_description("# Universal Coding Style\n\nbody"),
            Some("Universal Coding Style".to_string())
        );
    }

    #[test]
    fn extract_description_skips_leading_blank_lines() {
        let body = "\n\n   \n# Hello there\n\nbody";
        assert_eq!(extract_description(body), Some("Hello there".to_string()));
    }

    #[test]
    fn extract_description_returns_none_when_no_h1() {
        assert_eq!(extract_description("body without heading\n"), None);
        assert_eq!(extract_description("## H2 first\n# H1 later"), None);
        assert_eq!(extract_description(""), None);
    }

    #[test]
    fn extract_description_requires_space_after_hash() {
        // `#H1` (no space) is not a valid markdown H1 in any common
        // flavour. Treat it as non-heading prose so we don't surface a
        // bogus description.
        assert_eq!(extract_description("#NotAHeading\nbody"), None);
    }

    #[test]
    fn discover_finds_embedded_slugs_in_an_empty_config_dir() {
        let tmp = TempDir::new().unwrap();
        let map = discover(tmp.path()).unwrap();
        // Every shipped fixed-memory entry must appear with embedded-only
        // sources when the user has no overlay.
        assert!(map.contains_key("coding-style"));
        let entry = &map["coding-style"];
        assert!(entry.embedded.is_some());
        assert!(entry.user_path.is_none());
    }

    #[test]
    fn discover_treats_missing_user_dir_as_no_overlay() {
        let tmp = TempDir::new().unwrap();
        // No `fixed-memory/` subdir exists. Must succeed.
        let map = discover(tmp.path()).unwrap();
        assert!(!map.is_empty(), "embedded set should still populate");
    }

    #[test]
    fn discover_records_user_only_slug() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join(FIXED_MEMORY_DIR);
        fs::create_dir(&user_dir).unwrap();
        fs::write(
            user_dir.join("coding-style-haskell.md"),
            "# Haskell coding style\n",
        )
        .unwrap();

        let map = discover(tmp.path()).unwrap();
        let entry = &map["coding-style-haskell"];
        assert!(entry.embedded.is_none());
        assert!(entry.user_path.is_some());
    }

    #[test]
    fn discover_records_both_sources_when_slug_overlaps() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join(FIXED_MEMORY_DIR);
        fs::create_dir(&user_dir).unwrap();
        fs::write(user_dir.join("coding-style-rust.md"), "# my override\n").unwrap();

        let map = discover(tmp.path()).unwrap();
        let entry = &map["coding-style-rust"];
        assert!(entry.embedded.is_some());
        assert!(entry.user_path.is_some());
    }

    #[test]
    fn discover_silently_ignores_non_md_user_files() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join(FIXED_MEMORY_DIR);
        fs::create_dir(&user_dir).unwrap();
        fs::write(user_dir.join("README"), "not an entry\n").unwrap();
        fs::write(user_dir.join("notes.txt"), "also not\n").unwrap();

        let map = discover(tmp.path()).unwrap();
        assert!(!map.contains_key("README"));
        assert!(!map.contains_key("notes"));
    }

    #[test]
    fn compose_embedded_only_returns_embedded_unchanged() {
        let tmp = TempDir::new().unwrap();
        let body = compose("coding-style", tmp.path()).unwrap();
        assert!(!body.contains(ADDENDUM_DELIMITER));
        assert!(body.starts_with("# Universal Coding Style"));
    }

    #[test]
    fn compose_user_only_returns_user_unchanged() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join(FIXED_MEMORY_DIR);
        fs::create_dir(&user_dir).unwrap();
        fs::write(
            user_dir.join("coding-style-haskell.md"),
            "# Haskell coding style\nbody\n",
        )
        .unwrap();

        let body = compose("coding-style-haskell", tmp.path()).unwrap();
        assert_eq!(body, "# Haskell coding style\nbody\n");
        assert!(!body.contains(ADDENDUM_DELIMITER));
    }

    #[test]
    fn compose_both_layers_inserts_delimiter_between_them() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join(FIXED_MEMORY_DIR);
        fs::create_dir(&user_dir).unwrap();
        fs::write(
            user_dir.join("coding-style-rust.md"),
            "# my override\nuser body\n",
        )
        .unwrap();

        let body = compose("coding-style-rust", tmp.path()).unwrap();
        let split: Vec<&str> = body.split(ADDENDUM_DELIMITER).collect();
        assert_eq!(split.len(), 2, "delimiter must appear exactly once");
        assert!(split[0].contains("Rust Coding Style"));
        assert!(split[1].starts_with("# my override"));
    }

    #[test]
    fn compose_unknown_slug_returns_show_error_with_available_list() {
        let tmp = TempDir::new().unwrap();
        let err = compose("nope-not-real", tmp.path()).unwrap_err();
        match err {
            ShowError::UnknownSlug { slug, available } => {
                assert_eq!(slug, "nope-not-real");
                assert!(available.contains(&"coding-style".to_string()));
            }
            other => panic!("expected UnknownSlug, got {other:?}"),
        }
    }

    #[test]
    fn render_list_yaml_carries_schema_version_and_sources() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join(FIXED_MEMORY_DIR);
        fs::create_dir(&user_dir).unwrap();
        fs::write(
            user_dir.join("coding-style-haskell.md"),
            "# Haskell coding style\n",
        )
        .unwrap();
        let map = discover(tmp.path()).unwrap();

        let yaml = render_list(&map, OutputFormat::Yaml).unwrap();
        assert!(yaml.contains("schema_version: 1"));
        assert!(yaml.contains("slug: coding-style-haskell"));
        assert!(yaml.contains("- user"));
    }

    #[test]
    fn render_list_markdown_emits_table_header_and_rows() {
        let tmp = TempDir::new().unwrap();
        let map = discover(tmp.path()).unwrap();
        let md = render_list(&map, OutputFormat::Markdown).unwrap();
        assert!(md.starts_with("| slug | description | sources |\n|---|---|---|\n"));
        assert!(md.contains("| coding-style |"));
        assert!(md.contains("| embedded |"));
    }

    #[test]
    fn render_list_json_omits_description_when_absent() {
        let tmp = TempDir::new().unwrap();
        let user_dir = tmp.path().join(FIXED_MEMORY_DIR);
        fs::create_dir(&user_dir).unwrap();
        fs::write(user_dir.join("untitled.md"), "no heading here\n").unwrap();
        let map = discover(tmp.path()).unwrap();

        let json = render_list(&map, OutputFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entries = parsed["entries"].as_array().unwrap();
        let untitled = entries
            .iter()
            .find(|e| e["slug"] == "untitled")
            .expect("untitled slug present");
        assert!(
            untitled.get("description").is_none(),
            "description must be omitted when no H1 in either source"
        );
    }

    #[test]
    fn output_format_parse_accepts_known_values() {
        assert_eq!(OutputFormat::parse("yaml"), Some(OutputFormat::Yaml));
        assert_eq!(OutputFormat::parse("json"), Some(OutputFormat::Json));
        assert_eq!(
            OutputFormat::parse("markdown"),
            Some(OutputFormat::Markdown)
        );
        assert_eq!(OutputFormat::parse("toml"), None);
    }

    #[test]
    fn show_error_unknown_slug_message_names_remediation() {
        let err = ShowError::UnknownSlug {
            slug: "x".into(),
            available: vec!["a".into(), "b".into()],
        };
        let msg = format!("{err}");
        assert!(msg.contains("'x'") || msg.contains("\"x\""), "msg: {msg}");
        assert!(msg.contains("Available slugs: a, b"), "msg: {msg}");
        assert!(msg.contains("ravel-lite fixed-memory list"), "msg: {msg}");
    }
}
