//! Subtree-scoped git tree SHA for a project path.
//!
//! Works for both top-level repos and monorepo subtrees by computing
//! `rel = <project_path> relative to repo toplevel`, then running
//! `git rev-parse HEAD:<rel>`. An empty `rel` (project IS the repo)
//! returns the root tree.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Computes the subtree-scoped tree SHA for `project_path`.
///
/// Bails if:
/// * `project_path` is not inside a git repository.
/// * The subtree has uncommitted changes (using `git status --porcelain
///   -- <project_path>` from the repo toplevel).
pub fn compute_project_tree_sha(project_path: &Path) -> Result<String> {
    let toplevel = repo_toplevel(project_path)?;
    // Canonicalise so macOS symlinks like /var -> /private/var match git's
    // `--show-toplevel` output exactly.
    let canon_project = std::fs::canonicalize(project_path).with_context(|| {
        format!("failed to canonicalise project path {}", project_path.display())
    })?;
    let rel = canon_project
        .strip_prefix(&toplevel)
        .with_context(|| {
            format!(
                "project path {} is not a subpath of its git toplevel {}",
                canon_project.display(),
                toplevel.display()
            )
        })?;

    ensure_clean_subtree(&toplevel, rel)?;

    let spec = if rel.as_os_str().is_empty() {
        "HEAD^{tree}".to_string()
    } else {
        format!("HEAD:{}", rel.to_string_lossy())
    };

    let output = Command::new("git")
        .arg("-C")
        .arg(&toplevel)
        .arg("rev-parse")
        .arg(&spec)
        .output()
        .context("failed to spawn `git rev-parse`")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git rev-parse {} failed in {}: {}",
            spec,
            toplevel.display(),
            stderr.trim()
        );
    }
    let sha = String::from_utf8(output.stdout)
        .context("git rev-parse output was not valid UTF-8")?
        .trim()
        .to_string();
    if sha.is_empty() {
        bail!("git rev-parse {} returned empty output", spec);
    }
    Ok(sha)
}

fn repo_toplevel(project_path: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .context("failed to spawn `git rev-parse --show-toplevel`")?;
    if !output.status.success() {
        bail!(
            "project at {} is not inside a git repository — initialise with \
             `git init` or remove from the catalog",
            project_path.display()
        );
    }
    let s = String::from_utf8(output.stdout)
        .context("git --show-toplevel output was not valid UTF-8")?
        .trim()
        .to_string();
    Ok(PathBuf::from(s))
}

fn ensure_clean_subtree(toplevel: &Path, rel: &Path) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(toplevel).arg("status").arg("--porcelain");
    if !rel.as_os_str().is_empty() {
        cmd.arg("--").arg(rel);
    }
    let output = cmd
        .output()
        .context("failed to spawn `git status --porcelain`")?;
    if !output.status.success() {
        bail!(
            "git status --porcelain failed in {}",
            toplevel.display()
        );
    }
    let porcelain = String::from_utf8_lossy(&output.stdout);
    if !porcelain.trim().is_empty() {
        bail!(
            "project subtree at {} has uncommitted changes; commit or stash \
             before running discover:\n{}",
            rel.display(),
            porcelain.trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn init_repo_with_readme(path: &Path) {
        run(path, &["init", "-q", "-b", "main"]);
        run(path, &["config", "user.email", "test@example.com"]);
        run(path, &["config", "user.name", "test"]);
        std::fs::write(path.join("README.md"), "hello\n").unwrap();
        run(path, &["add", "README.md"]);
        run(path, &["commit", "-q", "-m", "init"]);
    }

    fn run(cwd: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(cwd).args(args).status().unwrap();
        assert!(status.success(), "git {:?} in {} failed", args, cwd.display());
    }

    #[test]
    fn top_level_repo_yields_non_empty_sha() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());

        let sha = compute_project_tree_sha(tmp.path()).unwrap();
        assert_eq!(sha.len(), 40, "expected 40-hex SHA, got {:?}", sha);
    }

    #[test]
    fn monorepo_subtrees_have_independent_shas() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());

        let sub_a = tmp.path().join("sub-a");
        let sub_b = tmp.path().join("sub-b");
        std::fs::create_dir_all(&sub_a).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();
        std::fs::write(sub_a.join("a.txt"), "A\n").unwrap();
        std::fs::write(sub_b.join("b.txt"), "B\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "add subs"]);

        let sha_a = compute_project_tree_sha(&sub_a).unwrap();
        let sha_b = compute_project_tree_sha(&sub_b).unwrap();
        assert_ne!(sha_a, sha_b, "subtrees with different content must have different SHAs");
    }

    #[test]
    fn sibling_subtree_change_does_not_invalidate_other_subtree() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        let sub_a = tmp.path().join("sub-a");
        let sub_b = tmp.path().join("sub-b");
        std::fs::create_dir_all(&sub_a).unwrap();
        std::fs::create_dir_all(&sub_b).unwrap();
        std::fs::write(sub_a.join("a.txt"), "A\n").unwrap();
        std::fs::write(sub_b.join("b.txt"), "B1\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "add subs"]);

        let sha_b_before = compute_project_tree_sha(&sub_b).unwrap();

        std::fs::write(sub_a.join("a.txt"), "A-edited\n").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "edit sub-a"]);

        let sha_b_after = compute_project_tree_sha(&sub_b).unwrap();
        assert_eq!(sha_b_before, sha_b_after, "sub-b's tree SHA must be stable across a commit that only touches sub-a");
    }

    #[test]
    fn non_git_project_bails_with_actionable_message() {
        let tmp = TempDir::new().unwrap();
        let err = compute_project_tree_sha(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not inside a git repository"), "got: {msg}");
        assert!(msg.contains("git init"), "got: {msg}");
    }

    #[test]
    fn dirty_subtree_bails() {
        let tmp = TempDir::new().unwrap();
        init_repo_with_readme(tmp.path());
        std::fs::write(tmp.path().join("README.md"), "edited\n").unwrap();

        let err = compute_project_tree_sha(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("uncommitted changes"), "got: {msg}");
    }
}
