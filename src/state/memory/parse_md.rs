//! Strict parser for the legacy `<plan>/memory.md` prose format.
//!
//! Used exclusively by the `state migrate` verb. Accepts the canonical
//! shape dream/reflect prompts emit today (`## <title>` headings followed
//! by a prose body, separated by blank lines) and refuses anything else.
//!
//! Any text before the first `## ` heading is discarded as preamble
//! (typically `# Memory` and a blank line); it carries no entry content.

use anyhow::{anyhow, bail, Result};

use crate::state::backlog::schema::allocate_id;

use super::schema::{MemoryEntry, MemoryFile};

pub fn parse_memory_markdown(input: &str) -> Result<MemoryFile> {
    let mut entries = Vec::new();
    let mut existing_ids: Vec<String> = Vec::new();

    for (block_index, block) in split_into_entry_blocks(input).into_iter().enumerate() {
        let entry = parse_single_entry_block(&block, &existing_ids).map_err(|err| {
            anyhow!("failed to parse memory entry #{}: {err:#}", block_index + 1)
        })?;
        existing_ids.push(entry.id.clone());
        entries.push(entry);
    }

    Ok(MemoryFile {
        entries,
        extra: Default::default(),
    })
}

fn split_into_entry_blocks(input: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in input.lines() {
        if line.starts_with("## ") {
            if let Some(buffer) = current.take() {
                if !buffer.trim().is_empty() {
                    blocks.push(buffer);
                }
            }
            current = Some(String::new());
        }
        if let Some(buf) = current.as_mut() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if let Some(buffer) = current {
        if !buffer.trim().is_empty() {
            blocks.push(buffer);
        }
    }
    blocks
}

fn parse_single_entry_block(block: &str, existing_ids: &[String]) -> Result<MemoryEntry> {
    let mut lines = block.lines();
    let title_line = lines.next().ok_or_else(|| anyhow!("empty memory entry block"))?;
    let title = title_line
        .strip_prefix("## ")
        .ok_or_else(|| anyhow!("entry block does not start with `## <title>`: {title_line:?}"))?
        .trim()
        .to_string();
    if title.is_empty() {
        bail!("memory entry title is empty");
    }

    let body_lines: Vec<&str> = lines.collect();
    // Trim leading blank lines (between the heading and the first body
    // paragraph) and trailing blank lines (before the next heading).
    let mut start = 0;
    while start < body_lines.len() && body_lines[start].trim().is_empty() {
        start += 1;
    }
    let mut end = body_lines.len();
    while end > start && body_lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    if start == end {
        bail!("memory entry {title:?} has no body");
    }
    let body = body_lines[start..end].join("\n") + "\n";

    let id = allocate_id(&title, existing_ids.iter().map(String::as_str));
    Ok(MemoryEntry { id, title, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_MEMORY: &str = "\
# Memory

## All prompt loading routes through `substitute_tokens`
Ad-hoc `str::replace` bypasses the hard-error guard regex. Drift guards require one canonical substitution path.

## Config overlays use deep-merge
`src/config.rs` implements `*.local.yaml` overlays. Scalar collisions go to overlay, map collisions recurse.
";

    #[test]
    fn parses_two_entries_skipping_top_level_header() {
        let memory = parse_memory_markdown(MINIMAL_MEMORY).unwrap();
        assert_eq!(memory.entries.len(), 2);
        assert_eq!(
            memory.entries[0].id,
            "all-prompt-loading-routes-through-substitute-tokens"
        );
        assert_eq!(
            memory.entries[0].title,
            "All prompt loading routes through `substitute_tokens`"
        );
        assert!(memory.entries[0].body.contains("str::replace"));
        assert_eq!(memory.entries[1].id, "config-overlays-use-deep-merge");
    }

    #[test]
    fn body_preserves_multiple_paragraphs() {
        let input = "\
# Memory

## Multi-paragraph entry

First paragraph.

Second paragraph with `code`.
";
        let memory = parse_memory_markdown(input).unwrap();
        assert_eq!(memory.entries.len(), 1);
        let body = &memory.entries[0].body;
        assert!(body.contains("First paragraph."));
        assert!(body.contains("Second paragraph with `code`."));
        // Blank line between paragraphs must be preserved.
        assert!(body.contains("\n\n"));
    }

    #[test]
    fn id_suffixes_on_title_collision() {
        let input = "\
# Memory

## Same title
First body.

## Same title
Second body.
";
        let memory = parse_memory_markdown(input).unwrap();
        assert_eq!(memory.entries[0].id, "same-title");
        assert_eq!(memory.entries[1].id, "same-title-2");
    }

    #[test]
    fn rejects_entry_with_empty_body() {
        let input = "\
# Memory

## Empty body entry

## Next entry
real body
";
        let err = parse_memory_markdown(input).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no body"), "error must mention empty body: {msg}");
    }

    #[test]
    fn empty_input_parses_to_zero_entries() {
        let memory = parse_memory_markdown("").unwrap();
        assert!(memory.entries.is_empty());
    }

    #[test]
    fn preamble_only_parses_to_zero_entries() {
        let memory = parse_memory_markdown("# Memory\n\nsome preamble\n").unwrap();
        assert!(memory.entries.is_empty());
    }

    #[test]
    fn parses_live_core_memory_without_error() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("LLM_STATE/core/memory.md");
        if !path.exists() {
            return;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let memory = parse_memory_markdown(&text).expect("core memory must parse");
        assert!(
            memory.entries.len() >= 10,
            "core memory must have many entries, got {}",
            memory.entries.len()
        );
    }
}
