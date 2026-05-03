//! Stable error-code vocabulary surfaced in JSON-mode error envelopes
//! (cli-tool-design.md §3). Typed-not-stringly per the project's
//! "Prefer typed APIs over stringly-typed" rule: a typo on the call
//! site is a compile error rather than a silent agent-branching break.
//!
//! Codes are ASCII-uppercase-snake-case strings in the wire protocol;
//! the round trip is `ErrorCode::<Variant>.as_str()` ↔
//! `ErrorCode::parse_opt(&str)`. New variants must extend this file —
//! no free-form codes downstream.

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum ErrorCode {
    /// Authentication or authorisation is required for the requested
    /// operation; the user has no valid credentials, or the credentials
    /// they have are not sufficient.
    AuthRequired,
    /// User-supplied input is syntactically or semantically invalid
    /// (bad flag value, malformed reference, unknown enum variant).
    InvalidInput,
    /// Named resource (id, slug, path) does not exist in the requested
    /// scope.
    NotFound,
    /// State on disk conflicts with the request (duplicate slug,
    /// schema_version mismatch, in-progress operation already holds the
    /// resource).
    Conflict,
    /// Filesystem or I/O failure (read/write/rename failed). Distinct
    /// from `Internal` because the remediation is usually about disk
    /// permissions or available space, not a code bug.
    IoError,
    /// External service or peer process throttled the request; the
    /// caller may retry after a backoff.
    RateLimited,
    /// Unexpected internal failure with no actionable remediation. The
    /// agent should NOT retry; this signals a code-level bug rather
    /// than a transient or user-fixable condition.
    Internal,
}

impl ErrorCode {
    /// Wire-form code, e.g. `"AUTH_REQUIRED"`. Stable across patch and
    /// minor versions; breaking changes go in major versions.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::AuthRequired => "AUTH_REQUIRED",
            ErrorCode::InvalidInput => "INVALID_INPUT",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Conflict => "CONFLICT",
            ErrorCode::IoError => "IO_ERROR",
            ErrorCode::RateLimited => "RATE_LIMITED",
            ErrorCode::Internal => "INTERNAL",
        }
    }

    /// Round-trip from the wire-form code. `None` for unknown inputs;
    /// callers that surface the parsed value to a user should explain
    /// the supported set (see [`ErrorCode::all`]).
    pub fn parse_opt(input: &str) -> Option<ErrorCode> {
        match input {
            "AUTH_REQUIRED" => Some(ErrorCode::AuthRequired),
            "INVALID_INPUT" => Some(ErrorCode::InvalidInput),
            "NOT_FOUND" => Some(ErrorCode::NotFound),
            "CONFLICT" => Some(ErrorCode::Conflict),
            "IO_ERROR" => Some(ErrorCode::IoError),
            "RATE_LIMITED" => Some(ErrorCode::RateLimited),
            "INTERNAL" => Some(ErrorCode::Internal),
            _ => None,
        }
    }

    /// The full vocabulary, in stable display order. Used by
    /// `ravel-lite capabilities` and any future schema documentation
    /// surface.
    pub fn all() -> &'static [ErrorCode] {
        &[
            ErrorCode::AuthRequired,
            ErrorCode::InvalidInput,
            ErrorCode::NotFound,
            ErrorCode::Conflict,
            ErrorCode::IoError,
            ErrorCode::RateLimited,
            ErrorCode::Internal,
        ]
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_every_variant() {
        for code in ErrorCode::all() {
            let s = code.as_str();
            let parsed = ErrorCode::parse_opt(s)
                .unwrap_or_else(|| panic!("ErrorCode::parse_opt failed for {s:?}"));
            assert_eq!(parsed, *code);
        }
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(ErrorCode::parse_opt("NO_SUCH_CODE"), None);
        assert_eq!(ErrorCode::parse_opt(""), None);
        assert_eq!(ErrorCode::parse_opt("auth_required"), None,
            "wire form is uppercase; lowercase must not match");
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(format!("{}", ErrorCode::AuthRequired), "AUTH_REQUIRED");
        assert_eq!(format!("{}", ErrorCode::Internal), "INTERNAL");
    }

    #[test]
    fn all_codes_are_unique() {
        let strs: Vec<&str> = ErrorCode::all().iter().map(ErrorCode::as_str).collect();
        let mut sorted = strs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(strs.len(), sorted.len(), "duplicate code in ErrorCode::all()");
    }
}
