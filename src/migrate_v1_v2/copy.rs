//! Half-A step 2: copy plan state files (yaml + phase.md) from the v1
//! plan dir to the new v2 plan dir. Stale `.md` siblings are skipped.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;

const FILES_TO_COPY: &[&str] = &[
    "phase.md",
    "backlog.yaml",
    "memory.yaml",
    "session-log.yaml",
    "latest-session.yaml",
];

pub fn copy_plan_state(old: &Path, new: &Path) -> Result<()> {
    fs::create_dir_all(new)
        .with_context(|| format!("create {}", new.display()))
        .with_code(ErrorCode::IoError)?;

    for name in FILES_TO_COPY {
        let src = old.join(name);
        if !src.is_file() {
            continue;
        }
        let dst = new.join(name);
        fs::copy(&src, &dst)
            .with_context(|| format!("copy {} -> {}", src.display(), dst.display()))
            .with_code(ErrorCode::IoError)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn copies_present_files_skips_missing_and_md_siblings() {
        let tmp = TempDir::new().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("phase.md"), "triage\n").unwrap();
        fs::write(old.join("backlog.yaml"), "schema_version: 1\nitems: []\n").unwrap();
        fs::write(old.join("memory.yaml"), "schema_version: 1\nitems: []\n").unwrap();
        // Stale .md siblings — must NOT be copied
        fs::write(old.join("backlog.md"), "stale\n").unwrap();
        fs::write(old.join("memory.md"), "stale\n").unwrap();

        copy_plan_state(&old, &new).unwrap();

        assert!(new.join("phase.md").is_file());
        assert!(new.join("backlog.yaml").is_file());
        assert!(new.join("memory.yaml").is_file());
        assert!(!new.join("backlog.md").exists(), "stale .md must not be copied");
        assert!(!new.join("memory.md").exists(), "stale .md must not be copied");
        assert!(
            !new.join("session-log.yaml").exists(),
            "missing files are skipped"
        );
    }
}
