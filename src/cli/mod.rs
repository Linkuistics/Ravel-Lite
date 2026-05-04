//! CLI-side concerns shared across every subcommand: output format
//! parsing, exit-code categories, error-code vocabulary, JSON-mode
//! error envelope, and other flag-level plumbing that does not belong
//! in any single per-kind module.

pub mod capabilities;
pub mod error_code;
pub mod error_context;
pub mod error_envelope;
pub mod exit_category;
pub mod list_limits;
pub mod output_format;

pub use error_code::ErrorCode;
pub use error_context::{error_code_of, CodedError, ResultExt};
pub use error_envelope::JsonErrorEnvelope;
pub use exit_category::ExitCategory;
pub use output_format::OutputFormat;
