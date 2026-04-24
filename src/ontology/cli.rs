//! Kebab-case parsers for the ontology enums used by CLI dispatchers.
//!
//! Two dispatchers (`state related-components` and
//! `state discover-proposals`) both accept kebab-case kind, lifecycle,
//! and evidence-grade arguments. The error messages enumerate the
//! legal vocabulary by reading `EdgeKind::all` / `LifecycleScope::all`,
//! so they stay in sync with the enum without drift tests.

use anyhow::{anyhow, Result};

use super::{EdgeKind, EvidenceGrade, LifecycleScope};

pub fn parse_edge_kind(input: &str) -> Result<EdgeKind> {
    EdgeKind::parse(input).ok_or_else(|| {
        let kebab = EdgeKind::all()
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow!("invalid kind {input:?}; expected one of the ontology v2 kinds: {kebab}")
    })
}

pub fn parse_lifecycle_scope(input: &str) -> Result<LifecycleScope> {
    LifecycleScope::parse(input).ok_or_else(|| {
        let kebab = LifecycleScope::all()
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow!("invalid lifecycle {input:?}; expected one of the ontology v2 lifecycles: {kebab}")
    })
}

pub fn parse_evidence_grade(input: &str) -> Result<EvidenceGrade> {
    EvidenceGrade::parse(input).ok_or_else(|| {
        anyhow!("invalid evidence-grade {input:?}; expected one of: strong, medium, weak")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_edge_kind_accepts_known_kebab() {
        let kind = parse_edge_kind("depends-on").unwrap();
        assert_eq!(kind.as_str(), "depends-on");
    }

    #[test]
    fn parse_edge_kind_rejects_unknown_and_lists_vocabulary() {
        let err = parse_edge_kind("bogus").unwrap_err().to_string();
        assert!(err.contains("invalid kind \"bogus\""), "err: {err}");
        for k in EdgeKind::all() {
            assert!(
                err.contains(k.as_str()),
                "error message missing vocabulary {:?}: {err}",
                k.as_str()
            );
        }
    }

    #[test]
    fn parse_lifecycle_scope_accepts_known_kebab() {
        let first = LifecycleScope::all()[0];
        let parsed = parse_lifecycle_scope(first.as_str()).unwrap();
        assert_eq!(parsed.as_str(), first.as_str());
    }

    #[test]
    fn parse_lifecycle_scope_rejects_unknown() {
        let err = parse_lifecycle_scope("bogus").unwrap_err().to_string();
        assert!(err.contains("invalid lifecycle"), "err: {err}");
    }

    #[test]
    fn parse_evidence_grade_accepts_all_three_kebab() {
        for grade in ["strong", "medium", "weak"] {
            assert_eq!(parse_evidence_grade(grade).unwrap().as_str(), grade);
        }
    }

    #[test]
    fn parse_evidence_grade_rejects_unknown() {
        let err = parse_evidence_grade("great").unwrap_err().to_string();
        assert!(err.contains("invalid evidence-grade"), "err: {err}");
    }
}
