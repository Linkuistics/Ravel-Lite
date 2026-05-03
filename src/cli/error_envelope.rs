//! JSON error envelope (cli-tool-design.md §3): when a verb fails and
//! the user is in `--format json` mode, the error must also be JSON so
//! the agent's parser does not need to mix prose stderr with structured
//! stdout.
//!
//! Envelope shape:
//!
//! ```json
//! {
//!   "error": {
//!     "code": "NOT_FOUND",
//!     "message": "no item with id \"b-foo\" in backlog",
//!     "remediation": "run `ravel-lite state backlog list <plan>` to inspect existing ids"
//!   }
//! }
//! ```
//!
//! `code` is always present (typed via [`ErrorCode`]). `message` is the
//! human-readable summary. `remediation` is `null` when no actionable
//! step is known — agents should NOT retry in that case (per §3:
//! "Errors that don't have one should say so, so the agent stops
//! retrying").

use serde::Serialize;

use crate::cli::error_code::ErrorCode;

/// Wire form of the JSON error envelope. Public so other modules can
/// construct one without a `Display` round trip; serialise via
/// `to_string` (which appends a trailing newline) so JSON-mode error
/// output framing matches every other JSON-emitting verb.
#[derive(Debug, Clone, Serialize)]
pub struct JsonErrorEnvelope {
    pub error: JsonErrorBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonErrorBody {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl JsonErrorEnvelope {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> JsonErrorEnvelope {
        JsonErrorEnvelope {
            error: JsonErrorBody {
                code: code.as_str(),
                message: message.into(),
                remediation: None,
            },
        }
    }

    pub fn with_remediation(mut self, remediation: impl Into<String>) -> JsonErrorEnvelope {
        self.error.remediation = Some(remediation.into());
        self
    }
}

/// Pretty-printed JSON with a trailing newline so output framing
/// matches every other JSON-emitting verb. Implemented as `Display`
/// (per clippy's `inherent_to_string` lint) so callers may write
/// `format!("{envelope}")` or pass the envelope to `print!` directly;
/// the `.to_string()` call site shape is preserved.
impl std::fmt::Display for JsonErrorEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match serde_json::to_string_pretty(self) {
            Ok(s) => {
                f.write_str(&s)?;
                f.write_str("\n")
            }
            // Should never fail: every field is a String/&'static str.
            Err(_) => writeln!(
                f,
                "{{\"error\":{{\"code\":\"{}\",\"message\":\"<envelope serialise failed>\"}}}}",
                self.error.code,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_envelope_omits_remediation_when_absent() {
        let json = JsonErrorEnvelope::new(ErrorCode::NotFound, "not here").to_string();
        assert!(json.contains("\"code\""));
        assert!(json.contains("NOT_FOUND"));
        assert!(json.contains("not here"));
        assert!(
            !json.contains("\"remediation\""),
            "absent remediation should be skipped, not rendered as null; got:\n{json}"
        );
        assert!(json.ends_with('\n'), "envelope must end with newline; got:\n{json}");
    }

    #[test]
    fn with_remediation_emits_the_field() {
        let json = JsonErrorEnvelope::new(ErrorCode::AuthRequired, "no creds")
            .with_remediation("run `ravel-lite auth login`")
            .to_string();
        assert!(json.contains("\"remediation\""));
        assert!(json.contains("ravel-lite auth login"));
    }

    #[test]
    fn json_is_well_formed_for_every_code() {
        for code in ErrorCode::all() {
            let env = JsonErrorEnvelope::new(*code, "msg");
            let text = env.to_string();
            let parsed: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("envelope for {code:?} did not round-trip: {e}; text:\n{text}"));
            let got_code = parsed["error"]["code"].as_str().expect("missing error.code");
            assert_eq!(got_code, code.as_str());
        }
    }

    #[test]
    fn envelope_message_quotes_are_escaped() {
        // Messages contain user-supplied ids that may include double quotes;
        // serde_json must escape them.
        let env = JsonErrorEnvelope::new(ErrorCode::NotFound, r#"id "b-foo" not found"#);
        let text = env.to_string();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["error"]["message"], r#"id "b-foo" not found"#);
    }
}
