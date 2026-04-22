//! Handlers for every `state memory <verb>` CLI verb.
//!
//! Dream-driven mutations are per-entry: add, set-body, set-title, delete.
//! There is no reorder verb; position is preserved implicitly through the
//! Vec order which dream mutates in place.

use std::path::Path;

use anyhow::{bail, Result};

use crate::state::backlog::schema::allocate_id;

use super::schema::{MemoryEntry, MemoryFile};
use super::yaml_io::{read_memory, write_memory};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OutputFormat {
    Yaml,
    Json,
}

impl OutputFormat {
    pub fn parse(input: &str) -> Option<OutputFormat> {
        match input {
            "yaml" => Some(OutputFormat::Yaml),
            "json" => Some(OutputFormat::Json),
            _ => None,
        }
    }
}

pub fn run_list(plan_dir: &Path, format: OutputFormat) -> Result<()> {
    let memory = read_memory(plan_dir)?;
    emit(&memory, format)
}

pub fn run_show(plan_dir: &Path, id: &str, format: OutputFormat) -> Result<()> {
    let memory = read_memory(plan_dir)?;
    let entry = find_entry(&memory, id)?;
    let wrapper = MemoryFile {
        entries: vec![entry.clone()],
        extra: Default::default(),
    };
    emit(&wrapper, format)
}

pub(crate) fn find_entry<'a>(memory: &'a MemoryFile, id: &str) -> Result<&'a MemoryEntry> {
    memory
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow::anyhow!("no memory entry with id {id:?}"))
}

fn emit(memory: &MemoryFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(memory)?,
        OutputFormat::Json => serde_json::to_string_pretty(memory)? + "\n",
    };
    print!("{serialised}");
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub title: String,
    pub body: String,
}

pub fn run_add(plan_dir: &Path, req: &AddRequest) -> Result<()> {
    let mut memory = read_memory(plan_dir)?;
    let id = allocate_id(&req.title, memory.entries.iter().map(|e| e.id.as_str()));
    memory.entries.push(MemoryEntry {
        id,
        title: req.title.clone(),
        body: ensure_trailing_newline(&req.body),
    });
    write_memory(plan_dir, &memory)
}

pub fn run_init(plan_dir: &Path, seed: &MemoryFile) -> Result<()> {
    let existing = read_memory(plan_dir)?;
    if !existing.entries.is_empty() {
        bail!(
            "refusing to init: memory.yaml at {} is non-empty ({} entries). Use `add` for incremental inserts.",
            plan_dir.display(),
            existing.entries.len()
        );
    }
    write_memory(plan_dir, seed)
}

pub fn run_set_body(plan_dir: &Path, id: &str, body: &str) -> Result<()> {
    let mut memory = read_memory(plan_dir)?;
    let entry = memory
        .entries
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow::anyhow!("no memory entry with id {id:?}"))?;
    entry.body = ensure_trailing_newline(body);
    write_memory(plan_dir, &memory)
}

pub fn run_set_title(plan_dir: &Path, id: &str, new_title: &str) -> Result<()> {
    let mut memory = read_memory(plan_dir)?;
    let entry = memory
        .entries
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow::anyhow!("no memory entry with id {id:?}"))?;
    entry.title = new_title.to_string();
    // id is intentionally preserved; slug stability matches the backlog convention.
    write_memory(plan_dir, &memory)
}

pub fn run_delete(plan_dir: &Path, id: &str) -> Result<()> {
    let mut memory = read_memory(plan_dir)?;
    let before = memory.entries.len();
    memory.entries.retain(|e| e.id != id);
    if memory.entries.len() == before {
        bail!("no memory entry with id {id:?}");
    }
    write_memory(plan_dir, &memory)
}

fn ensure_trailing_newline(body: &str) -> String {
    if body.ends_with('\n') {
        body.to_string()
    } else {
        format!("{body}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_memory() -> MemoryFile {
        MemoryFile {
            entries: vec![
                MemoryEntry {
                    id: "foo".into(),
                    title: "Foo entry".into(),
                    body: "Foo body.\n".into(),
                },
                MemoryEntry {
                    id: "bar".into(),
                    title: "Bar entry".into(),
                    body: "Bar body.\n".into(),
                },
            ],
            extra: Default::default(),
        }
    }

    #[test]
    fn find_entry_returns_entry_by_id() {
        let memory = sample_memory();
        let entry = find_entry(&memory, "bar").unwrap();
        assert_eq!(entry.id, "bar");
    }

    #[test]
    fn find_entry_errors_when_id_not_found() {
        let memory = sample_memory();
        let err = find_entry(&memory, "nonexistent").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must include bad id: {msg}");
    }

    #[test]
    fn run_add_appends_entry_with_allocated_id() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        let req = AddRequest {
            title: "New Entry".into(),
            body: "Body of new entry.\n".into(),
        };
        run_add(tmp.path(), &req).unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        assert_eq!(updated.entries.last().unwrap().id, "new-entry");
        assert_eq!(updated.entries.last().unwrap().title, "New Entry");
    }

    #[test]
    fn run_add_suffixes_on_id_collision() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        let req = AddRequest {
            title: "Foo entry".into(),
            body: "Body.\n".into(),
        };
        run_add(tmp.path(), &req).unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        assert_eq!(updated.entries.last().unwrap().id, "foo-entry");
        // Pre-existing "foo" was short-slug; the new one has a distinct slug,
        // so no numeric suffix is needed. This test guards the allocator
        // integration, not the suffix branch specifically.
    }

    #[test]
    fn run_init_populates_empty_memory() {
        let tmp = TempDir::new().unwrap();
        write_memory(
            tmp.path(),
            &MemoryFile {
                entries: vec![],
                extra: Default::default(),
            },
        )
        .unwrap();

        run_init(tmp.path(), &sample_memory()).unwrap();

        let stored = read_memory(tmp.path()).unwrap();
        assert_eq!(stored.entries.len(), 2);
    }

    #[test]
    fn run_init_refuses_to_overwrite_non_empty_memory() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        let err = run_init(tmp.path(), &sample_memory()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("non-empty"), "error must explain refusal: {msg}");
    }

    #[test]
    fn run_set_body_rewrites_the_entry_body() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        run_set_body(tmp.path(), "foo", "Rewritten body.\n").unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        let foo = updated.entries.iter().find(|e| e.id == "foo").unwrap();
        assert_eq!(foo.body, "Rewritten body.\n");
    }

    #[test]
    fn run_set_title_updates_title_but_preserves_id() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        run_set_title(tmp.path(), "bar", "Bar's New Title").unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        let bar = updated.entries.iter().find(|e| e.id == "bar").unwrap();
        assert_eq!(bar.title, "Bar's New Title");
        assert_eq!(bar.id, "bar", "id must not change when title changes");
    }

    #[test]
    fn run_delete_removes_entry_by_id() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        run_delete(tmp.path(), "foo").unwrap();

        let updated = read_memory(tmp.path()).unwrap();
        assert_eq!(updated.entries.len(), 1);
        assert_eq!(updated.entries[0].id, "bar");
    }

    #[test]
    fn run_delete_errors_on_unknown_id() {
        let tmp = TempDir::new().unwrap();
        write_memory(tmp.path(), &sample_memory()).unwrap();

        let err = run_delete(tmp.path(), "nonexistent").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("nonexistent"), "error must cite the bad id: {msg}");
    }
}
