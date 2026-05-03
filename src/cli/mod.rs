//! CLI-side concerns shared across every subcommand: output format
//! parsing, exit-code categories (forthcoming), and other flag-level
//! plumbing that does not belong in any single per-kind module.

pub mod output_format;

pub use output_format::OutputFormat;
