//! Single shared `OutputFormat` enum used by every `--format` flag in the
//! CLI surface. Replaces the per-kind enums that previously lived in each
//! `state::*::verbs` module, `fixed_memory`, and `plan_inspect`.
//!
//! All three variants are universal at the type level. Renderers that do
//! not support a given variant return an actionable error at runtime
//! rather than rejecting the flag at parse time — the user sees "format X
//! is not supported on `<verb>`; supported: …" instead of the bare
//! "invalid --format value".

use anyhow::{anyhow, Result};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OutputFormat {
    Yaml,
    Json,
    Markdown,
}

impl OutputFormat {
    pub fn parse_opt(input: &str) -> Option<OutputFormat> {
        match input {
            "yaml" => Some(OutputFormat::Yaml),
            "json" => Some(OutputFormat::Json),
            "markdown" => Some(OutputFormat::Markdown),
            _ => None,
        }
    }

    /// Parse a `--format` argument. Returns an actionable error naming the
    /// supported set when the input is unrecognised.
    pub fn parse(input: &str) -> Result<OutputFormat> {
        Self::parse_opt(input).ok_or_else(|| {
            anyhow!("invalid --format value {input:?}; expected `yaml`, `json`, or `markdown`")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_variants() {
        assert_eq!(OutputFormat::parse_opt("yaml"), Some(OutputFormat::Yaml));
        assert_eq!(OutputFormat::parse_opt("json"), Some(OutputFormat::Json));
        assert_eq!(
            OutputFormat::parse_opt("markdown"),
            Some(OutputFormat::Markdown)
        );
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(OutputFormat::parse_opt("xml"), None);
        assert_eq!(OutputFormat::parse_opt(""), None);
    }

    #[test]
    fn parse_error_names_supported_set() {
        let err = OutputFormat::parse("xml").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("yaml"), "{msg}");
        assert!(msg.contains("json"), "{msg}");
        assert!(msg.contains("markdown"), "{msg}");
    }
}
