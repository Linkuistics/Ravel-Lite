//! Plumbing for tagging errors with a typed [`ErrorCode`] so the
//! exit-category and JSON-envelope `code` field land on every error
//! that has a known classification, instead of falling through to the
//! catch-all [`ErrorCode::Internal`].
//!
//! ## How it works
//!
//! [`CodedError`] is a small struct that carries a code plus the
//! human-readable message. It implements `std::error::Error` so
//! `anyhow` accepts it, and its [`Display`] impl emits only the
//! message — the code is metadata that the renderer in `main()`
//! recovers via `anyhow::Error::downcast_ref::<CodedError>()`.
//!
//! ## Call-site forms
//!
//! `bail_with!` is the direct equivalent of `anyhow::bail!` with a
//! mandatory leading code:
//!
//! ```ignore
//! use ravel_lite::cli::ErrorCode;
//! use ravel_lite::bail_with;
//!
//! bail_with!(ErrorCode::NotFound, "no item with id {id:?} in backlog");
//! ```
//!
//! [`ResultExt::with_code`] upgrades an existing `Result<_, anyhow::Error>`
//! (typically the result of a `?`-propagated IO/parse error) so the
//! tag survives:
//!
//! ```ignore
//! use ravel_lite::cli::error_context::ResultExt;
//! use ravel_lite::cli::ErrorCode;
//!
//! let body = std::fs::read_to_string(&path)
//!     .map_err(anyhow::Error::from)
//!     .with_code(ErrorCode::IoError)?;
//! ```
//!
//! ## Recovery
//!
//! Use [`error_code_of`] from `main()` to pull the code out before
//! rendering the JSON envelope or computing the exit category.
//!
//! ## Design note
//!
//! `with_code` collapses an existing chained anyhow error into a
//! single message string. CLI-level errors are read by humans or
//! agents as a single rendered message, so flattening is acceptable;
//! preserving the chain typed-end-to-end would require either
//! changing every dispatch signature or a more invasive newtype
//! wrapper that re-exports `Source`. The trade-off here favours
//! locality: every coded error is a plain `CodedError` value, easily
//! inspected and asserted against in tests.

use std::fmt;

use crate::cli::error_code::ErrorCode;

/// Error type that carries an [`ErrorCode`] alongside a flattened
/// human-readable message.
#[derive(Debug)]
pub struct CodedError {
    pub code: ErrorCode,
    pub message: String,
}

impl fmt::Display for CodedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodedError {}

/// Recover the [`ErrorCode`] tagged on this error, or [`ErrorCode::Internal`]
/// when no tag is present. Walks the anyhow chain — works regardless
/// of whether the `CodedError` is the root of the chain or wrapped by
/// later context layers.
pub fn error_code_of(err: &anyhow::Error) -> ErrorCode {
    err.downcast_ref::<CodedError>()
        .map(|c| c.code)
        .unwrap_or(ErrorCode::Internal)
}

/// Extension trait for `Result<T, anyhow::Error>` that attaches a
/// typed code. Flattens any existing chain into the message — see the
/// module-level design note.
pub trait ResultExt<T> {
    fn with_code(self, code: ErrorCode) -> Result<T, anyhow::Error>;
}

impl<T> ResultExt<T> for Result<T, anyhow::Error> {
    fn with_code(self, code: ErrorCode) -> Result<T, anyhow::Error> {
        self.map_err(|err| {
            anyhow::Error::new(CodedError {
                code,
                message: format!("{err:#}"),
            })
        })
    }
}

/// `bail!`-flavoured macro that attaches an [`ErrorCode`] before
/// returning. Equivalent to:
///
/// ```ignore
/// return Err(anyhow::Error::new(CodedError { code, message: format!(...) }));
/// ```
#[macro_export]
macro_rules! bail_with {
    ($code:expr, $($arg:tt)*) => {
        return ::std::result::Result::Err(::anyhow::Error::new(
            $crate::cli::error_context::CodedError {
                code: $code,
                message: ::std::format!($($arg)*),
            },
        ))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bail_with_attaches_recoverable_code() {
        fn produces_error() -> anyhow::Result<()> {
            bail_with!(ErrorCode::NotFound, "no item {id:?}", id = "b-foo");
        }

        let err = produces_error().unwrap_err();
        assert_eq!(error_code_of(&err), ErrorCode::NotFound);
        assert!(format!("{err:#}").contains("b-foo"));
    }

    #[test]
    fn untagged_error_falls_back_to_internal() {
        let err = anyhow::anyhow!("plain error with no code"); // errorcode-exempt: test asserts the untagged-fallback contract
        assert_eq!(error_code_of(&err), ErrorCode::Internal);
    }

    #[test]
    fn result_ext_with_code_overrides_internal_default() {
        let result: Result<(), anyhow::Error> = Err(anyhow::anyhow!("disk read failed")); // errorcode-exempt: test exercises the .with_code() upgrade path
        let coded = result.with_code(ErrorCode::IoError).unwrap_err();
        assert_eq!(error_code_of(&coded), ErrorCode::IoError);
        assert!(format!("{coded:#}").contains("disk read failed"));
    }

    #[test]
    fn coded_error_display_shows_only_the_message() {
        // The Display impl must not include the code — agents read the
        // message verbatim. The code is metadata in the JSON envelope
        // and exit category, never in the prose.
        let ce = CodedError {
            code: ErrorCode::AuthRequired,
            message: "credentials missing".to_string(),
        };
        let rendered = format!("{ce}");
        assert_eq!(rendered, "credentials missing");
        assert!(!rendered.contains("AUTH_REQUIRED"));
    }

    #[test]
    fn coded_error_remains_recoverable_after_with_context_wrap() {
        // Caller wraps a CodedError-bearing Result with .with_context();
        // error_code_of must still find the inner code by walking the
        // anyhow chain. This is the contract that lets parser internals
        // bail with a typed code while their callers add narrative
        // context without losing the classification.
        use anyhow::Context;

        let inner: Result<(), anyhow::Error> = Err(anyhow::Error::new(CodedError {
            code: ErrorCode::InvalidInput,
            message: "bad heading".to_string(),
        }));
        let wrapped = inner.with_context(|| "failed to parse session-log.md");
        let err = wrapped.unwrap_err();
        assert_eq!(error_code_of(&err), ErrorCode::InvalidInput);
        let rendered = format!("{err:#}");
        assert!(rendered.contains("failed to parse session-log.md"));
        assert!(rendered.contains("bad heading"));
    }

    #[test]
    fn every_error_code_round_trips_through_a_coded_error() {
        for code in ErrorCode::all() {
            let err: anyhow::Error = anyhow::Error::new(CodedError {
                code: *code,
                message: "test".to_string(),
            });
            assert_eq!(error_code_of(&err), *code);
        }
    }
}
