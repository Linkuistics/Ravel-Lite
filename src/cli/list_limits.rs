//! `--limit N` / `--all` truncation infrastructure for `list`-style verbs.
//!
//! Per `defaults/fixed-memory/cli-tool-design.md` §6, every verb that emits
//! an unbounded list must support pagination so an agent's context isn't
//! blown out by a single `list` call. This module owns the shared pieces
//! every verb needs:
//!
//! - [`ListLimits`] — the clap-friendly flag pair (`--limit N` / `--all`).
//! - [`ListEnvelope`] — the YAML/JSON output wrapper. When truncation
//!   does not apply, the envelope serialises as `{ schema_version, items }`
//!   only, so untruncated output is byte-identical to the on-disk file
//!   shape (backward-compatible with pre-pagination consumers).
//! - [`apply`] — pure function that takes the full slice and the limits,
//!   returns the envelope plus a truncation flag callers use to decide
//!   whether to emit the human stderr line.
//!
//! Per-verb defaults are passed in by callers: per-plan listings (memory,
//! backlog, intents) typically pass `default_limit = None` so behaviour
//! is unbounded unless the user opts in via `--limit`. Inherently large
//! listings (`atlas list-components`, `state session-log list`) can pass
//! a real default to opt their callers into safe-by-default truncation.

use serde::Serialize;

/// Clap flag-pair for list pagination.
///
/// `limit` and `all` are mutually exclusive at the clap layer (declared
/// via `conflicts_with` on the `--all` flag at each verb's call site).
/// This type intentionally does *not* derive `clap::Args` — clap-side
/// declarations are repeated per-verb so each verb's `--help` carries
/// the documentation where users will read it. This type is the
/// post-parse value carrier consumed by [`apply`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ListLimits {
    pub limit: Option<usize>,
    pub all: bool,
}

impl ListLimits {
    /// The effective truncation cap. `None` means unbounded.
    ///
    /// Resolution order:
    /// - `--all` → unbounded.
    /// - `--limit N` → at most `N`.
    /// - Neither → `default_limit` (verb-supplied; `None` means unbounded).
    pub fn effective(&self, default_limit: Option<usize>) -> Option<usize> {
        if self.all {
            None
        } else {
            self.limit.or(default_limit)
        }
    }
}

/// Output wrapper for list verbs.
///
/// The shape `{ schema_version, items }` mirrors every on-disk
/// `*File` struct in `src/state/`. The optional `truncated` /
/// `total` / `returned` fields appear only when truncation actually
/// applied; an untruncated envelope serialises identically to the
/// original file shape, preserving backward compatibility with any
/// consumer that round-trips list output through the on-disk schema.
#[derive(Debug, Clone, Serialize)]
pub struct ListEnvelope<T: Serialize> {
    pub schema_version: u32,
    pub items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned: Option<usize>,
}

/// Apply pagination to an items slice, producing a [`ListEnvelope`].
///
/// `default_limit` is the per-verb default cap (verbs that should be
/// safe-by-default pass `Some(N)`; verbs that should default to the
/// historical unbounded behaviour pass `None`). When the resolved cap
/// is `None` or `total <= cap`, no truncation fields are emitted; the
/// caller's existing untruncated stdout shape is preserved.
pub fn apply<T: Clone + Serialize>(
    full: &[T],
    limits: &ListLimits,
    default_limit: Option<usize>,
    schema_version: u32,
) -> ListEnvelope<T> {
    let total = full.len();
    let cap = limits.effective(default_limit);
    match cap {
        None => untruncated(full.to_vec(), schema_version),
        Some(n) if total <= n => untruncated(full.to_vec(), schema_version),
        Some(n) => ListEnvelope {
            schema_version,
            items: full.iter().take(n).cloned().collect(),
            truncated: Some(true),
            total: Some(total),
            returned: Some(n),
        },
    }
}

fn untruncated<T: Serialize>(items: Vec<T>, schema_version: u32) -> ListEnvelope<T> {
    ListEnvelope {
        schema_version,
        items,
        truncated: None,
        total: None,
        returned: None,
    }
}

/// Human-readable truncation summary.
///
/// Returns the stderr line callers should emit (without trailing newline)
/// when the envelope was truncated. The line is fixed-format and matches
/// the example in `cli-tool-design.md` §6 so agents that prose-match it
/// see consistent text across every verb.
pub fn truncation_summary_line<T: Serialize>(env: &ListEnvelope<T>) -> Option<String> {
    let total = env.total?;
    let returned = env.returned?;
    Some(format!(
        "Showing {returned} of {total} results. Use --limit N or --all to see more."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_unbounded_when_neither_flag_set_and_no_default() {
        let limits = ListLimits {
            limit: None,
            all: false,
        };
        assert_eq!(limits.effective(None), None);
    }

    #[test]
    fn effective_uses_explicit_limit_over_default() {
        let limits = ListLimits {
            limit: Some(7),
            all: false,
        };
        assert_eq!(limits.effective(Some(100)), Some(7));
    }

    #[test]
    fn effective_uses_default_when_no_explicit_limit() {
        let limits = ListLimits {
            limit: None,
            all: false,
        };
        assert_eq!(limits.effective(Some(100)), Some(100));
    }

    #[test]
    fn effective_all_overrides_explicit_default() {
        let limits = ListLimits {
            limit: None,
            all: true,
        };
        assert_eq!(limits.effective(Some(100)), None);
    }

    #[test]
    fn effective_all_overrides_explicit_limit() {
        // clap rejects this combination at parse time but the type's
        // semantics still need to be defined: --all wins.
        let limits = ListLimits {
            limit: Some(5),
            all: true,
        };
        assert_eq!(limits.effective(None), None);
    }

    #[test]
    fn apply_unbounded_preserves_full_list_with_no_metadata() {
        let env = apply(&[1, 2, 3], &ListLimits::default(), None, 1);
        assert_eq!(env.schema_version, 1);
        assert_eq!(env.items, vec![1, 2, 3]);
        assert_eq!(env.truncated, None);
        assert_eq!(env.total, None);
        assert_eq!(env.returned, None);
    }

    #[test]
    fn apply_truncates_when_total_exceeds_limit() {
        let limits = ListLimits {
            limit: Some(3),
            all: false,
        };
        let env = apply(&[10, 20, 30, 40, 50], &limits, None, 1);
        assert_eq!(env.items, vec![10, 20, 30]);
        assert_eq!(env.truncated, Some(true));
        assert_eq!(env.total, Some(5));
        assert_eq!(env.returned, Some(3));
    }

    #[test]
    fn apply_does_not_truncate_when_total_equals_limit() {
        let limits = ListLimits {
            limit: Some(3),
            all: false,
        };
        let env = apply(&[1, 2, 3], &limits, None, 1);
        assert_eq!(env.items, vec![1, 2, 3]);
        assert_eq!(env.truncated, None);
        assert_eq!(env.total, None);
        assert_eq!(env.returned, None);
    }

    #[test]
    fn apply_does_not_truncate_when_total_below_limit() {
        let limits = ListLimits {
            limit: Some(10),
            all: false,
        };
        let env = apply(&[1, 2], &limits, None, 1);
        assert_eq!(env.items, vec![1, 2]);
        assert_eq!(env.truncated, None);
    }

    #[test]
    fn apply_default_limit_kicks_in_when_no_explicit_limit() {
        let env = apply(
            &[1, 2, 3, 4, 5],
            &ListLimits::default(),
            Some(2),
            1,
        );
        assert_eq!(env.items, vec![1, 2]);
        assert_eq!(env.truncated, Some(true));
        assert_eq!(env.total, Some(5));
        assert_eq!(env.returned, Some(2));
    }

    #[test]
    fn apply_all_overrides_default_limit() {
        let limits = ListLimits {
            limit: None,
            all: true,
        };
        let env = apply(&[1, 2, 3, 4, 5], &limits, Some(2), 1);
        assert_eq!(env.items, vec![1, 2, 3, 4, 5]);
        assert_eq!(env.truncated, None);
    }

    #[test]
    fn apply_empty_input_produces_empty_envelope_with_no_metadata() {
        let env = apply::<i32>(&[], &ListLimits::default(), Some(10), 1);
        assert!(env.items.is_empty());
        assert_eq!(env.truncated, None);
    }

    #[test]
    fn untruncated_envelope_yaml_contains_only_schema_version_and_items() {
        let env = apply(&[1, 2, 3], &ListLimits::default(), None, 1);
        let yaml = serde_yaml::to_string(&env).unwrap();
        // Only the two backwards-compatible fields appear; truncation
        // metadata is suppressed so consumers that round-trip through
        // an existing `*File` struct still parse the output cleanly.
        assert!(yaml.contains("schema_version: 1"), "yaml: {yaml}");
        assert!(yaml.contains("items:"), "yaml: {yaml}");
        assert!(!yaml.contains("truncated"), "yaml must not carry truncation metadata when not truncated: {yaml}");
        assert!(!yaml.contains("total"), "yaml must not carry truncation metadata when not truncated: {yaml}");
        assert!(!yaml.contains("returned"), "yaml must not carry truncation metadata when not truncated: {yaml}");
    }

    #[test]
    fn truncated_envelope_yaml_carries_metadata_fields() {
        let limits = ListLimits {
            limit: Some(2),
            all: false,
        };
        let env = apply(&[1, 2, 3], &limits, None, 1);
        let yaml = serde_yaml::to_string(&env).unwrap();
        assert!(yaml.contains("truncated: true"), "yaml: {yaml}");
        assert!(yaml.contains("total: 3"), "yaml: {yaml}");
        assert!(yaml.contains("returned: 2"), "yaml: {yaml}");
    }

    #[test]
    fn truncated_envelope_json_carries_metadata_fields() {
        let limits = ListLimits {
            limit: Some(2),
            all: false,
        };
        let env = apply(&[1, 2, 3], &limits, None, 1);
        let json = serde_json::to_string(&env).unwrap();
        assert!(json.contains("\"truncated\":true"), "json: {json}");
        assert!(json.contains("\"total\":3"), "json: {json}");
        assert!(json.contains("\"returned\":2"), "json: {json}");
    }

    #[test]
    fn truncation_summary_line_present_when_truncated() {
        let limits = ListLimits {
            limit: Some(2),
            all: false,
        };
        let env = apply(&[1, 2, 3, 4, 5], &limits, None, 1);
        let line = truncation_summary_line(&env).expect("truncated envelope must produce a line");
        assert!(line.contains("Showing 2 of 5"), "line: {line}");
        assert!(line.contains("--all"), "line must cite --all remediation: {line}");
    }

    #[test]
    fn truncation_summary_line_absent_when_not_truncated() {
        let env = apply(&[1, 2, 3], &ListLimits::default(), None, 1);
        assert!(truncation_summary_line(&env).is_none());
    }
}
