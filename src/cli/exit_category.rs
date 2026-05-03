//! Documented exit-code categories (cli-tool-design.md §8). Typed-not-
//! stringly: the integer-to-meaning mapping is owned by this enum so
//! call sites cannot drift.
//!
//! The mapping is a stable contract — agents branch on `$?` to
//! distinguish "your fault" (usage errors, not-found, auth) from
//! "system fault" (IO, internal) without parsing stderr. Keep the
//! integers stable across versions; introduce new categories rather
//! than renumbering existing ones.

use crate::cli::error_code::ErrorCode;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum ExitCategory {
    /// `0` — the operation completed.
    Success,
    /// `1` — generic failure, no specific category applies. Agents
    /// should treat this as opaque; retry is unsafe.
    GenericFailure,
    /// `2` — bad flags, missing required argument, or other parse-time
    /// usage error.
    UsageError,
    /// `3` — a referenced id/slug/path does not exist in scope.
    NotFound,
    /// `4` — authentication or authorisation is required.
    AuthRequired,
    /// `5` — preconditions failed, resource conflict, or schema-version
    /// mismatch.
    Conflict,
    /// `6` — external service throttled the request; safe to retry
    /// after a backoff.
    RateLimited,
}

impl ExitCategory {
    /// Process exit code for this category. Stable across versions.
    pub fn as_code(&self) -> i32 {
        match self {
            ExitCategory::Success => 0,
            ExitCategory::GenericFailure => 1,
            ExitCategory::UsageError => 2,
            ExitCategory::NotFound => 3,
            ExitCategory::AuthRequired => 4,
            ExitCategory::Conflict => 5,
            ExitCategory::RateLimited => 6,
        }
    }

    /// One-line label suitable for help-text rendering.
    pub fn label(&self) -> &'static str {
        match self {
            ExitCategory::Success => "success",
            ExitCategory::GenericFailure => "generic failure",
            ExitCategory::UsageError => "usage error",
            ExitCategory::NotFound => "not found",
            ExitCategory::AuthRequired => "auth required / forbidden",
            ExitCategory::Conflict => "conflict / precondition failed",
            ExitCategory::RateLimited => "rate limited / try again later",
        }
    }

    /// All non-success categories in code order. Used to render the
    /// exit-code table in top-level `--help`.
    pub fn documented() -> &'static [ExitCategory] {
        &[
            ExitCategory::Success,
            ExitCategory::GenericFailure,
            ExitCategory::UsageError,
            ExitCategory::NotFound,
            ExitCategory::AuthRequired,
            ExitCategory::Conflict,
            ExitCategory::RateLimited,
        ]
    }
}

impl From<&ErrorCode> for ExitCategory {
    fn from(code: &ErrorCode) -> ExitCategory {
        match code {
            ErrorCode::AuthRequired => ExitCategory::AuthRequired,
            ErrorCode::InvalidInput => ExitCategory::UsageError,
            ErrorCode::NotFound => ExitCategory::NotFound,
            ErrorCode::Conflict => ExitCategory::Conflict,
            ErrorCode::RateLimited => ExitCategory::RateLimited,
            ErrorCode::IoError => ExitCategory::GenericFailure,
            ErrorCode::Internal => ExitCategory::GenericFailure,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(ExitCategory::Success.as_code(), 0);
        assert_eq!(ExitCategory::GenericFailure.as_code(), 1);
        assert_eq!(ExitCategory::UsageError.as_code(), 2);
        assert_eq!(ExitCategory::NotFound.as_code(), 3);
        assert_eq!(ExitCategory::AuthRequired.as_code(), 4);
        assert_eq!(ExitCategory::Conflict.as_code(), 5);
        assert_eq!(ExitCategory::RateLimited.as_code(), 6);
    }

    #[test]
    fn from_error_code_maps_every_variant() {
        for code in ErrorCode::all() {
            let cat = ExitCategory::from(code);
            assert_ne!(cat.as_code(), ExitCategory::Success.as_code(),
                "ErrorCode {code:?} must not map to Success");
        }
    }

    #[test]
    fn from_error_code_specific_mappings() {
        assert_eq!(ExitCategory::from(&ErrorCode::AuthRequired), ExitCategory::AuthRequired);
        assert_eq!(ExitCategory::from(&ErrorCode::InvalidInput), ExitCategory::UsageError);
        assert_eq!(ExitCategory::from(&ErrorCode::NotFound), ExitCategory::NotFound);
        assert_eq!(ExitCategory::from(&ErrorCode::Conflict), ExitCategory::Conflict);
        assert_eq!(ExitCategory::from(&ErrorCode::RateLimited), ExitCategory::RateLimited);
        assert_eq!(ExitCategory::from(&ErrorCode::IoError), ExitCategory::GenericFailure);
        assert_eq!(ExitCategory::from(&ErrorCode::Internal), ExitCategory::GenericFailure);
    }

    #[test]
    fn documented_is_dense_from_zero() {
        let codes: Vec<i32> = ExitCategory::documented().iter().map(ExitCategory::as_code).collect();
        assert_eq!(codes, vec![0, 1, 2, 3, 4, 5, 6],
            "documented() must be dense from 0 so the help-text table is contiguous");
    }
}
