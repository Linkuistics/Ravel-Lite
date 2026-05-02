//! Typed intents.yaml surface and CRUD CLI verbs.

pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use schema::{IntentEntry, IntentsFile, INTENTS_SCHEMA_VERSION};
pub use verbs::{run_add, run_list, run_set_status, run_show, AddRequest, OutputFormat};
pub use yaml_io::{intents_path, read_intents, write_intents};
