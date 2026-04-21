// src/dream.rs
use std::fs;
use std::path::Path;

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Returns true if memory.md has grown beyond baseline + headroom.
pub fn should_dream(plan_dir: &Path, headroom: usize) -> bool {
    let memory_path = plan_dir.join("memory.md");
    let baseline_path = plan_dir.join("dream-baseline");

    let Ok(memory) = fs::read_to_string(&memory_path) else {
        return false;
    };
    let Ok(baseline_str) = fs::read_to_string(&baseline_path) else {
        return false;
    };
    let Ok(baseline) = baseline_str.trim().parse::<usize>() else {
        return false;
    };

    word_count(&memory) > baseline + headroom
}

/// Update the dream baseline to the current word count of memory.md.
pub fn update_dream_baseline(plan_dir: &Path) {
    let memory_path = plan_dir.join("memory.md");
    let baseline_path = plan_dir.join("dream-baseline");

    if let Ok(memory) = fs::read_to_string(&memory_path) {
        let count = word_count(&memory);
        let _ = fs::write(&baseline_path, count.to_string());
    }
}

/// Self-healing seed: write `dream-baseline=0` when the file is
/// missing. Idempotent — no-op if the baseline already exists.
///
/// `0` means "we have never successfully dreamed." This matches the
/// value `src/create.rs` writes at plan creation, so the file's initial
/// contract is identical regardless of who produced it. Dream fires
/// once memory.md exceeds `headroom` words; after a dream runs,
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
    use tempfile::TempDir;

    #[test]
    fn returns_false_when_no_memory() {
        let dir = TempDir::new().unwrap();
        assert!(!should_dream(dir.path(), 1500));
    }

    #[test]
    fn returns_false_when_no_baseline() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("memory.md"), "hello world").unwrap();
        assert!(!should_dream(dir.path(), 1500));
    }

    #[test]
    fn returns_false_within_headroom() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("memory.md"), "word ".repeat(100)).unwrap();
        fs::write(dir.path().join("dream-baseline"), "50").unwrap();
        assert!(!should_dream(dir.path(), 1500));
    }

    #[test]
    fn returns_true_beyond_headroom() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("memory.md"), "word ".repeat(2000)).unwrap();
        fs::write(dir.path().join("dream-baseline"), "100").unwrap();
        assert!(should_dream(dir.path(), 1500));
    }

    #[test]
    fn update_baseline_writes_word_count() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("memory.md"), "word ".repeat(500)).unwrap();
        update_dream_baseline(dir.path());
        let baseline = fs::read_to_string(dir.path().join("dream-baseline")).unwrap();
        assert_eq!(baseline.trim().parse::<usize>().unwrap(), 500);
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
        fs::write(dir.path().join("memory.md"), "word ".repeat(2000)).unwrap();
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
        fs::write(dir.path().join("memory.md"), "word ".repeat(500)).unwrap();
        fs::write(dir.path().join("dream-baseline"), "42").unwrap();

        seed_dream_baseline_if_missing(dir.path());

        let baseline = fs::read_to_string(dir.path().join("dream-baseline")).unwrap();
        assert_eq!(baseline.trim(), "42");
    }
}
