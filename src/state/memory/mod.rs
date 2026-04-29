//! Typed memory.yaml surface and CRUD CLI verbs.

pub mod parse_md;
pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use parse_md::parse_memory_markdown;
pub use schema::{MemoryEntry, MemoryFile, MEMORY_SCHEMA_VERSION};
pub use verbs::{
    run_add, run_delete, run_init, run_list, run_set_body, run_set_status, run_set_title,
    run_show, AddRequest, OutputFormat,
};
pub use yaml_io::{read_memory, write_memory};
