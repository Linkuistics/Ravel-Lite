//! `state migrate <plan-dir>` — single-plan conversion of legacy .md
//! files into typed .yaml siblings.
//!
//! R1 scope: backlog.md only. Future rollouts (R2–R3) extend this verb
//! in place to cover memory, session-log, latest-session, and phase.
//! Does not touch related-plans.md (handled by the separate
//! migrate-related-projects verb when R5 lands).

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::state::backlog::{
    parse_backlog_markdown, read_backlog, write_backlog, BacklogFile,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OriginalPolicy {
    Keep,
    Delete,
}

#[derive(Debug, Clone)]
pub struct MigrateOptions {
    pub dry_run: bool,
    pub original_policy: OriginalPolicy,
    pub force: bool,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        MigrateOptions {
            dry_run: false,
            original_policy: OriginalPolicy::Keep,
            force: false,
        }
    }
}

pub fn run_migrate(plan_dir: &Path, options: &MigrateOptions) -> Result<()> {
    let source = plan_dir.join("backlog.md");
    let target = plan_dir.join("backlog.yaml");

    if !source.exists() {
        bail!(
            "no backlog.md to migrate at {}. Either the plan has no backlog or migration has already run.",
            source.display()
        );
    }

    let text = std::fs::read_to_string(&source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    let parsed = parse_backlog_markdown(&text)
        .with_context(|| format!("failed to parse {} as legacy backlog markdown", source.display()))?;

    // Idempotency: if the target exists, require the re-migration output
    // to match the current file content (modulo canonical serialisation).
    if target.exists() {
        let existing = read_backlog(plan_dir)
            .with_context(|| "failed to read existing backlog.yaml for idempotency check")?;
        if backlogs_equivalent(&existing, &parsed) {
            if matches!(options.original_policy, OriginalPolicy::Delete) && !options.dry_run {
                std::fs::remove_file(&source)
                    .with_context(|| format!("failed to delete {}", source.display()))?;
            }
            return Ok(()); // no-op
        }
        if !options.force {
            bail!(
                "{} already exists and differs from the re-migration output. Rerun with --force to overwrite.",
                target.display()
            );
        }
    }

    if options.dry_run {
        println!("dry-run: would write {} ({} tasks)", target.display(), parsed.tasks.len());
        if matches!(options.original_policy, OriginalPolicy::Delete) {
            println!("dry-run: would delete {}", source.display());
        }
        return Ok(());
    }

    write_backlog(plan_dir, &parsed)?;

    // Validation round-trip: re-parse the file we just wrote and assert
    // it round-trips to the same BacklogFile content.
    let validated = read_backlog(plan_dir)
        .with_context(|| "validation round-trip read failed after write")?;
    if !backlogs_equivalent(&validated, &parsed) {
        bail!(
            "validation mismatch: backlog.yaml re-read does not match the parse result. Aborting without deleting the original."
        );
    }

    if matches!(options.original_policy, OriginalPolicy::Delete) {
        std::fs::remove_file(&source)
            .with_context(|| format!("failed to delete {}", source.display()))?;
    }
    Ok(())
}

/// Structural equivalence for idempotency / validation checks. Ignores
/// the `extra` IndexMap because a re-migration always emits empty extra
/// (no unknown top-level keys in a legacy-markdown parse).
fn backlogs_equivalent(a: &BacklogFile, b: &BacklogFile) -> bool {
    if a.tasks.len() != b.tasks.len() {
        return false;
    }
    for (task_a, task_b) in a.tasks.iter().zip(b.tasks.iter()) {
        if task_a.id != task_b.id
            || task_a.title != task_b.title
            || task_a.category != task_b.category
            || task_a.status != task_b.status
            || task_a.blocked_reason != task_b.blocked_reason
            || task_a.dependencies != task_b.dependencies
            || task_a.description != task_b.description
            || task_a.results != task_b.results
            || task_a.handoff != task_b.handoff
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const TWO_TASK_MARKDOWN: &str = "\
### First task

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

First task body.

**Results:** _pending_

---

### Second task

**Category:** `research`
**Status:** `done`
**Dependencies:** First task

**Description:**

Second task body.

**Results:**

Done and dusted.

---
";

    fn write_md(plan: &Path, content: &str) {
        std::fs::write(plan.join("backlog.md"), content).unwrap();
    }

    #[test]
    fn migrate_writes_backlog_yaml_and_keeps_md_by_default() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        assert!(tmp.path().join("backlog.yaml").exists());
        assert!(tmp.path().join("backlog.md").exists(), "default is keep-originals");

        let backlog = read_backlog(tmp.path()).unwrap();
        assert_eq!(backlog.tasks.len(), 2);
    }

    #[test]
    fn migrate_with_delete_originals_removes_md_after_success() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);

        let opts = MigrateOptions {
            original_policy: OriginalPolicy::Delete,
            ..MigrateOptions::default()
        };
        run_migrate(tmp.path(), &opts).unwrap();

        assert!(tmp.path().join("backlog.yaml").exists());
        assert!(!tmp.path().join("backlog.md").exists(), "md must be deleted on success");
    }

    #[test]
    fn migrate_dry_run_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);

        let opts = MigrateOptions {
            dry_run: true,
            ..MigrateOptions::default()
        };
        run_migrate(tmp.path(), &opts).unwrap();

        assert!(!tmp.path().join("backlog.yaml").exists(), "dry-run must not write");
        assert!(tmp.path().join("backlog.md").exists());
    }

    #[test]
    fn migrate_is_idempotent_on_second_run() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);

        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();
        // Second run must no-op — no error, no changes.
        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        let backlog = read_backlog(tmp.path()).unwrap();
        assert_eq!(backlog.tasks.len(), 2);
    }

    #[test]
    fn migrate_refuses_overwrite_on_diverged_yaml_without_force() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), TWO_TASK_MARKDOWN);
        run_migrate(tmp.path(), &MigrateOptions::default()).unwrap();

        // Tamper with the yaml so it diverges from the markdown.
        let mut backlog = read_backlog(tmp.path()).unwrap();
        backlog.tasks[0].title = "Tampered title".into();
        write_backlog(tmp.path(), &backlog).unwrap();

        let err = run_migrate(tmp.path(), &MigrateOptions::default()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("already exists"), "error must mention existence: {msg}");
        assert!(msg.contains("--force"), "error must cite --force: {msg}");

        // With --force, the tampered yaml is overwritten.
        let opts = MigrateOptions { force: true, ..MigrateOptions::default() };
        run_migrate(tmp.path(), &opts).unwrap();
        let backlog = read_backlog(tmp.path()).unwrap();
        assert_eq!(backlog.tasks[0].title, "First task");
    }

    #[test]
    fn migrate_parse_failure_leaves_filesystem_untouched() {
        let tmp = TempDir::new().unwrap();
        write_md(tmp.path(), "### Malformed task\n\nno category or status\n");

        let err = run_migrate(tmp.path(), &MigrateOptions::default()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Category") || msg.contains("Status"), "error must name the missing field: {msg}");
        assert!(!tmp.path().join("backlog.yaml").exists(), "partial writes forbidden on parse failure");
    }
}
