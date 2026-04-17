// src/git.rs
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

pub struct CommitResult {
    pub committed: bool,
    pub message: String,
}

/// Stage plan directory and commit with the message from commit-message.md
/// (or a default message). Returns whether anything was committed.
pub fn git_commit_plan(plan_dir: &Path, plan_name: &str, phase_name: &str) -> Result<CommitResult> {
    let commit_msg_path = plan_dir.join("commit-message.md");
    let message = if commit_msg_path.exists() {
        let msg = fs::read_to_string(&commit_msg_path)
            .context("Failed to read commit-message.md")?
            .trim()
            .to_string();
        fs::remove_file(&commit_msg_path).ok();
        msg
    } else {
        format!("run-plan: {phase_name} ({plan_name})")
    };

    Command::new("git")
        .current_dir(plan_dir)
        .args(["add", "."])
        .output()
        .context("Failed to run git add")?;

    let diff = Command::new("git")
        .current_dir(plan_dir)
        .args(["diff", "--cached", "--quiet"])
        .output()
        .context("Failed to run git diff")?;

    if diff.status.success() {
        return Ok(CommitResult {
            committed: false,
            message,
        });
    }

    Command::new("git")
        .current_dir(plan_dir)
        .args(["commit", "-m", &message])
        .output()
        .context("Failed to run git commit")?;

    Ok(CommitResult {
        committed: true,
        message,
    })
}

/// Save the current HEAD sha as the work baseline.
pub fn git_save_work_baseline(plan_dir: &Path) {
    let baseline_path = plan_dir.join("work-baseline");
    let sha = Command::new("git")
        .current_dir(plan_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default()
        .trim()
        .to_string();
    let _ = fs::write(&baseline_path, &sha);
}

/// Lines from `git status --porcelain` run from `project_dir`. Each entry
/// is the raw porcelain line including the two-character XY status prefix
/// — preserved so the caller can render them identically to what the user
/// would see if they ran `git status` themselves.
///
/// Used by the work-phase commit boundary as a postcondition: a clean
/// project tree after the work commit means the agent committed
/// everything it claimed; non-empty output means something was edited
/// but not committed (the silent-failure mode that masks lost work as
/// "backlog empty"). Returns `Ok(vec![])` on a clean tree.
pub fn working_tree_status(project_dir: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .current_dir(project_dir)
        .args(["status", "--porcelain"])
        .output()
        .context("Failed to run git status")?;
    if !output.status.success() {
        anyhow::bail!(
            "git status exited {} in {}",
            output.status,
            project_dir.display()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// Find the project root by walking up from a directory to find .git.
pub fn find_project_root(start_dir: &Path) -> Result<String> {
    let mut dir = start_dir.canonicalize().unwrap_or_else(|_| start_dir.to_path_buf());
    loop {
        if dir.join(".git").exists() {
            return Ok(dir.to_string_lossy().to_string());
        }
        if !dir.pop() {
            anyhow::bail!("No .git found above {}", start_dir.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_project_root_finds_git() {
        // This test runs inside a git repo (the raveloop project itself)
        let result = find_project_root(Path::new("."));
        assert!(result.is_ok());
    }

    #[test]
    fn find_project_root_errors_on_root() {
        let result = find_project_root(Path::new("/tmp/nonexistent-asdhjkasd"));
        assert!(result.is_err());
    }

    #[test]
    fn working_tree_status_reports_dirty_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path();
        Command::new("git").current_dir(repo).args(["init", "-q"]).output().unwrap();
        // git init must succeed before we can stage anything; minimal config so commits work.
        Command::new("git").current_dir(repo).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.name", "t"]).output().unwrap();

        // Untracked file shows up as ?? in porcelain output.
        fs::write(repo.join("dirty.txt"), "x").unwrap();
        let status = working_tree_status(repo).unwrap();
        assert!(
            status.iter().any(|l| l.contains("dirty.txt")),
            "expected dirty.txt in porcelain output, got: {status:?}"
        );
    }

    #[test]
    fn working_tree_status_empty_on_clean_tree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path();
        Command::new("git").current_dir(repo).args(["init", "-q"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.email", "t@t"]).output().unwrap();
        Command::new("git").current_dir(repo).args(["config", "user.name", "t"]).output().unwrap();
        // Empty repo with no untracked files — porcelain output should be empty.
        let status = working_tree_status(repo).unwrap();
        assert!(status.is_empty(), "expected empty status on clean tree, got: {status:?}");
    }
}
