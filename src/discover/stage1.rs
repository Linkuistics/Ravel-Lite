//! Stage 1: per-project interaction-surface extraction.
//!
//! For each project in the catalog:
//!   1. Compute its subtree-scoped tree SHA.
//!   2. If the cached surface's `tree_sha` matches, use it as-is.
//!   3. Otherwise, spawn a `claude -p` subagent with CWD = project path
//!      and the Stage 1 prompt; parse YAML output; inject identity
//!      fields; write cache atomically.
//!
//! Dispatch is bounded by a `tokio::sync::Semaphore`; default 4.
//!
//! Failure policy is best-effort: per-project failures are captured in
//! a `Vec<Stage1Failure>` and surfaced in the proposals file. They do
//! not abort the pipeline.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::projects::ProjectEntry;

use super::cache;
use super::schema::{Stage1Failure, SurfaceFile, SurfaceRecord, SURFACE_SCHEMA_VERSION};
use super::tree_sha::compute_project_tree_sha;

pub const DEFAULT_STAGE1_TIMEOUT_SECS: u64 = 600;

/// The concrete outcome for one project in a Stage 1 pass.
#[derive(Debug)]
pub enum Stage1Outcome {
    Fresh(SurfaceFile),
    Cached(SurfaceFile),
    Failed(Stage1Failure),
}

pub struct Stage1Config {
    pub config_root: PathBuf,
    pub model: String,
    pub prompt_template: String,
    pub concurrency: usize,
    pub timeout: Duration,
}

pub async fn run_stage1(
    projects: &[ProjectEntry],
    cfg: &Stage1Config,
) -> Result<Vec<Stage1Outcome>> {
    let semaphore = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
    let mut join_set: JoinSet<(String, Result<Stage1Outcome>)> = JoinSet::new();

    for project in projects {
        let permit_sem = Arc::clone(&semaphore);
        let config_root = cfg.config_root.clone();
        let model = cfg.model.clone();
        let prompt_template = cfg.prompt_template.clone();
        let timeout = cfg.timeout;
        let name = project.name.clone();
        let path = project.path.clone();

        join_set.spawn(async move {
            // Acquire permit; a closed semaphore here is a bug (we never
            // close it), so surface it as an error rather than unwrapping.
            let outcome = match permit_sem.acquire_owned().await {
                Ok(_permit) => {
                    process_project(
                        &config_root,
                        &name,
                        &path,
                        &model,
                        &prompt_template,
                        timeout,
                    )
                    .await
                }
                Err(e) => Err(anyhow::anyhow!("semaphore closed unexpectedly: {e}")),
            };
            (name, outcome)
        });
    }

    let mut outcomes = Vec::with_capacity(projects.len());
    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok((_name, Ok(outcome))) => outcomes.push(outcome),
            Ok((name, Err(e))) => outcomes.push(Stage1Outcome::Failed(Stage1Failure {
                project: name,
                error: format!("{e:#}"),
            })),
            Err(join_err) => outcomes.push(Stage1Outcome::Failed(Stage1Failure {
                project: "<unknown>".to_string(),
                error: format!("join error: {join_err:#}"),
            })),
        }
    }
    Ok(outcomes)
}

async fn process_project(
    config_root: &Path,
    name: &str,
    path: &Path,
    model: &str,
    prompt_template: &str,
    timeout: Duration,
) -> Result<Stage1Outcome> {
    let tree_sha = compute_project_tree_sha(path).with_context(|| {
        format!(
            "compute_project_tree_sha for '{name}' at {}",
            path.display()
        )
    })?;

    if let Some(cached) = cache::load(config_root, name)? {
        if cached.tree_sha == tree_sha {
            return Ok(Stage1Outcome::Cached(cached));
        }
    }

    let cache_dir = cache::cache_dir(config_root);
    std::fs::create_dir_all(&cache_dir).with_context(|| {
        format!("failed to create cache dir {}", cache_dir.display())
    })?;
    let output_path = cache_dir.join(format!(".tmp-{name}-{}.yaml", std::process::id()));
    if output_path.exists() {
        std::fs::remove_file(&output_path).with_context(|| {
            format!("failed to remove stale tmp file {}", output_path.display())
        })?;
    }

    let prompt = prompt_template.replace(
        "{{SURFACE_OUTPUT_PATH}}",
        &output_path.to_string_lossy(),
    );

    let exit_ok = spawn_claude_with_cwd(&prompt, model, path, timeout).await?;
    if !exit_ok {
        bail!("Stage 1 subagent for '{name}' exited non-zero");
    }
    if !output_path.exists() {
        bail!(
            "Stage 1 subagent for '{name}' did not create {}",
            output_path.display()
        );
    }

    let raw = std::fs::read_to_string(&output_path).with_context(|| {
        format!("failed to read Stage 1 output {}", output_path.display())
    })?;
    let surface: SurfaceRecord = serde_yaml::from_str(&raw).with_context(|| {
        format!(
            "parse Stage 1 output for '{name}' from {}",
            output_path.display()
        )
    })?;
    let _ = std::fs::remove_file(&output_path);

    let file = SurfaceFile {
        schema_version: SURFACE_SCHEMA_VERSION,
        project: name.to_string(),
        tree_sha: tree_sha.clone(),
        analysed_at: current_utc_rfc3339(),
        surface,
    };
    cache::save_atomic(config_root, &file)?;
    Ok(Stage1Outcome::Fresh(file))
}

async fn spawn_claude_with_cwd(
    prompt: &str,
    model: &str,
    cwd: &Path,
    timeout: Duration,
) -> Result<bool> {
    let mut child = TokioCommand::new("claude")
        .arg("-p")
        .arg(prompt)
        .arg("--model")
        .arg(model)
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("project,local")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn `claude` — ensure it is installed and on PATH")?;

    let mut stdout = child
        .stdout
        .take()
        .context("claude stdout pipe unavailable")?;
    let mut drain = String::new();
    let wait = tokio::time::timeout(timeout, async {
        let _ = stdout.read_to_string(&mut drain).await;
        child.wait().await
    })
    .await;
    match wait {
        Ok(Ok(status)) => Ok(status.success()),
        Ok(Err(io_err)) => Err(io_err).context("waiting on claude process"),
        Err(_elapsed) => {
            let _ = child.kill().await;
            bail!(
                "claude Stage 1 subagent timed out after {}s in {}",
                timeout.as_secs(),
                cwd.display()
            )
        }
    }
}

/// Render the current UTC time as an RFC-3339 string (`YYYY-MM-DDTHH:MM:SSZ`).
///
/// We roll our own rather than pull in `chrono` just for a timestamp —
/// the surface cache's `analysed_at` is informational, not parsed back.
fn current_utc_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_utc(secs)
}

/// Convert a Unix-epoch second count into `YYYY-MM-DDTHH:MM:SSZ`.
/// Uses the proleptic Gregorian calendar; handles dates from 1970 onward.
fn format_unix_utc(mut secs: u64) -> String {
    let seconds = (secs % 60) as u32;
    secs /= 60;
    let minutes = (secs % 60) as u32;
    secs /= 60;
    let hours = (secs % 24) as u32;
    let mut days = secs / 24;

    // Advance year by year, subtracting each year's days.
    let mut year: u32 = 1970;
    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if days < year_days as u64 {
            break;
        }
        days -= year_days as u64;
        year += 1;
    }

    // Advance month by month.
    let month_lens: [u32; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month: u32 = 0;
    while month < 12 && days >= month_lens[month as usize] as u64 {
        days -= month_lens[month as usize] as u64;
        month += 1;
    }
    let day = (days as u32) + 1;
    let month_1based = month + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month_1based, day, hours, minutes, seconds
    )
}

fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::TempDir;

    use super::super::schema::SurfaceRecord;
    use super::*;

    /// Local copy of the git fixture used by `tree_sha::tests`. We can't
    /// reach across the module's private `tests` submodule, so the
    /// duplication is deliberate and narrow.
    fn init_repo_with_readme(path: &Path) {
        run_git(path, &["init", "-q", "-b", "main"]);
        run_git(path, &["config", "user.email", "test@example.com"]);
        run_git(path, &["config", "user.name", "test"]);
        std::fs::write(path.join("README.md"), "hello\n").unwrap();
        run_git(path, &["add", "README.md"]);
        run_git(path, &["commit", "-q", "-m", "init"]);
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} in {} failed", args, cwd.display());
    }

    /// Directly exercise `process_project` against a project whose cache
    /// is already warm with the current tree SHA — it must return
    /// `Cached(..)` without attempting to spawn claude.
    #[tokio::test]
    async fn cache_hit_bypasses_subagent() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();

        // Make a fresh git repo for the project, compute its real SHA.
        let project = tmp.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        init_repo_with_readme(&project);
        let sha = compute_project_tree_sha(&project).unwrap();

        // Seed a cache entry with the exact SHA.
        let file = SurfaceFile {
            schema_version: SURFACE_SCHEMA_VERSION,
            project: "Proj".to_string(),
            tree_sha: sha.clone(),
            analysed_at: "2026-04-22T00:00:00Z".to_string(),
            surface: SurfaceRecord {
                purpose: "cached".to_string(),
                ..Default::default()
            },
        };
        cache::save_atomic(&cfg, &file).unwrap();

        let outcome = process_project(
            &cfg,
            "Proj",
            &project,
            "unused-model",
            "unused-prompt",
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        match outcome {
            Stage1Outcome::Cached(f) => {
                assert_eq!(f.tree_sha, sha);
                assert_eq!(f.surface.purpose, "cached");
            }
            other => panic!("expected Cached outcome, got {other:?}"),
        }
    }

    #[test]
    fn format_unix_utc_known_timestamps() {
        // 1970-01-01T00:00:00Z
        assert_eq!(format_unix_utc(0), "1970-01-01T00:00:00Z");
        // 2000-01-01T00:00:00Z (includes Y2K leap-year handling)
        assert_eq!(format_unix_utc(946_684_800), "2000-01-01T00:00:00Z");
        // 2024-01-01T00:00:00Z (leap year boundary)
        assert_eq!(format_unix_utc(1_704_067_200), "2024-01-01T00:00:00Z");
        // 2024-02-29T12:34:56Z (leap-day handling)
        assert_eq!(format_unix_utc(1_709_210_096), "2024-02-29T12:34:56Z");
    }
}
