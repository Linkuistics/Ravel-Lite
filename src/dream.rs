// src/dream.rs
use std::fs;
use std::path::Path;

use crate::state::memory::read_memory;

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Sum of words across every parsed memory entry's title and body.
/// Returns `None` when `memory.yaml` is absent or fails to parse —
/// treated as "nothing to dream about" by the callers.
fn memory_content_word_count(plan_dir: &Path) -> Option<usize> {
    let memory = read_memory(plan_dir).ok()?;
    let mut total = 0;
    for entry in &memory.entries {
        total += word_count(&entry.title);
        total += word_count(&entry.body);
    }
    Some(total)
}

/// Returns true if memory content has grown beyond baseline + headroom.
pub fn should_dream(plan_dir: &Path, headroom: usize) -> bool {
    let baseline_path = plan_dir.join("dream-baseline");

    let Ok(baseline_str) = fs::read_to_string(&baseline_path) else {
        return false;
    };
    let Ok(baseline) = baseline_str.trim().parse::<usize>() else {
        return false;
    };
    let Some(count) = memory_content_word_count(plan_dir) else {
        return false;
    };

    count > baseline + headroom
}

/// Update the dream baseline to the current word count of memory content.
pub fn update_dream_baseline(plan_dir: &Path) {
    let baseline_path = plan_dir.join("dream-baseline");

    if let Some(count) = memory_content_word_count(plan_dir) {
        let _ = fs::write(&baseline_path, count.to_string());
    }
}

/// Self-healing seed: write `dream-baseline=0` when the file is
/// missing. Idempotent — no-op if the baseline already exists.
///
/// `0` means "we have never successfully dreamed." This matches the
/// value `src/create.rs` writes at plan creation, so the file's initial
/// contract is identical regardless of who produced it. Dream fires
/// once memory content exceeds `headroom` words; after a dream runs,
/// `update_dream_baseline` captures the post-compaction count as the
/// new baseline.
///
/// Called from three layers for defense-in-depth: plan creation,
/// every `ravel-lite state set-phase` invocation, and the
/// `GitCommitReflect` handler. Any one layer firing is sufficient, so
/// a deleted or never-created baseline self-repairs on the next
/// phase transition.
pub fn seed_dream_baseline_if_missing(plan_dir: &Path) {
    let baseline_path = plan_dir.join("dream-baseline");
    if baseline_path.exists() {
        return;
    }
    let _ = fs::write(&baseline_path, "0");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::memory::{write_memory, MemoryFile};
    use crate::state::memory::schema::MemoryEntry;
    use tempfile::TempDir;

    /// Seed a `memory.yaml` whose parsed content word count equals
    /// `target_words` (one entry, empty title, body of exactly that
    /// many space-separated tokens). Keeps tests focused on threshold
    /// behaviour without depending on any particular YAML encoding.
    fn write_memory_with_word_count(dir: &Path, target_words: usize) {
        let body = if target_words == 0 {
            String::new()
        } else {
            vec!["word"; target_words].join(" ")
        };
        let memory = MemoryFile {
            entries: vec![MemoryEntry {
                id: "test-entry".into(),
                title: String::new(),
                body,
            }],
            extra: Default::default(),
        };
        write_memory(dir, &memory).unwrap();
    }

    #[test]
    fn returns_false_when_no_memory() {
        let dir = TempDir::new().unwrap();
        assert!(!should_dream(dir.path(), 1500));
    }

    #[test]
    fn returns_false_when_no_baseline() {
        let dir = TempDir::new().unwrap();
        write_memory_with_word_count(dir.path(), 2);
        assert!(!should_dream(dir.path(), 1500));
    }

    #[test]
    fn returns_false_within_headroom() {
        let dir = TempDir::new().unwrap();
        write_memory_with_word_count(dir.path(), 100);
        fs::write(dir.path().join("dream-baseline"), "50").unwrap();
        assert!(!should_dream(dir.path(), 1500));
    }

    #[test]
    fn returns_true_beyond_headroom() {
        let dir = TempDir::new().unwrap();
        write_memory_with_word_count(dir.path(), 2000);
        fs::write(dir.path().join("dream-baseline"), "100").unwrap();
        assert!(should_dream(dir.path(), 1500));
    }

    #[test]
    fn update_baseline_writes_word_count() {
        let dir = TempDir::new().unwrap();
        write_memory_with_word_count(dir.path(), 500);
        update_dream_baseline(dir.path());
        let baseline = fs::read_to_string(dir.path().join("dream-baseline")).unwrap();
        assert_eq!(baseline.trim().parse::<usize>().unwrap(), 500);
    }

    #[test]
    fn update_baseline_counts_entry_titles_and_bodies() {
        // Regression: word count must cover both title and body of
        // each memory entry. Counting body only would under-report by
        // exactly the title-word budget, silently moving the dream
        // threshold.
        let dir = TempDir::new().unwrap();
        let memory = MemoryFile {
            entries: vec![
                MemoryEntry {
                    id: "a".into(),
                    title: "Alpha title words".into(), // 3 words
                    body: "alpha body word one two".into(), // 5 words
                },
                MemoryEntry {
                    id: "b".into(),
                    title: "Beta".into(), // 1 word
                    body: "".into(), // 0 words
                },
            ],
            extra: Default::default(),
        };
        write_memory(dir.path(), &memory).unwrap();
        update_dream_baseline(dir.path());
        let baseline = fs::read_to_string(dir.path().join("dream-baseline")).unwrap();
        assert_eq!(baseline.trim().parse::<usize>().unwrap(), 9);
    }

    #[test]
    fn seed_writes_zero_when_baseline_missing_regardless_of_memory_size() {
        // Semantic: a missing baseline means "we have never dreamed,"
        // so seed to 0 regardless of whether memory is empty or
        // already-populated. Populated-memory case is the important
        // one — seeding to current word count would delay the first
        // dream by `headroom` words on plans that pre-date the
        // fallback, effectively freezing them at their current size.
        let dir = TempDir::new().unwrap();
        write_memory_with_word_count(dir.path(), 2000);
        assert!(!dir.path().join("dream-baseline").exists());

        seed_dream_baseline_if_missing(dir.path());

        let baseline = fs::read_to_string(dir.path().join("dream-baseline")).unwrap();
        assert_eq!(baseline.trim(), "0");
    }

    #[test]
    fn seed_is_noop_when_baseline_already_exists() {
        // Idempotence: the seed must not clobber a baseline written by
        // update_dream_baseline (post-dream) or by plan-creation.
        // Otherwise every phase transition would reset the baseline
        // and dream could never be reached.
        let dir = TempDir::new().unwrap();
        write_memory_with_word_count(dir.path(), 500);
        fs::write(dir.path().join("dream-baseline"), "42").unwrap();

        seed_dream_baseline_if_missing(dir.path());

        let baseline = fs::read_to_string(dir.path().join("dream-baseline")).unwrap();
        assert_eq!(baseline.trim(), "42");
    }
}
