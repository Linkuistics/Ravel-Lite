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
use super::tree_sha::compute_project_state;

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
    /// Names of every project in the catalog. Used to substitute
    /// `{{CATALOG_PROJECTS}}` in the prompt with the catalog minus the
    /// project currently being analysed, so the subagent can scope
    /// `explicit_cross_project_mentions` to first-party catalog entries.
    pub catalog_names: Vec<String>,
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
        let catalog_names = cfg.catalog_names.clone();
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
                        &catalog_names,
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
    catalog_names: &[String],
    timeout: Duration,
) -> Result<Stage1Outcome> {
    let state = compute_project_state(path).with_context(|| {
        format!(
            "compute_project_state for '{name}' at {}",
            path.display()
        )
    })?;

    if let Some(cached) = cache::load(config_root, name)? {
        if cached.tree_sha == state.tree_sha && cached.dirty_hash == state.dirty_hash {
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

    let catalog_block = render_catalog_for_prompt(name, catalog_names);
    let prompt = prompt_template
        .replace("{{SURFACE_OUTPUT_PATH}}", &output_path.to_string_lossy())
        .replace("{{CATALOG_PROJECTS}}", &catalog_block);

    let exit_ok = spawn_claude_with_cwd(&prompt, model, path, &cache_dir, timeout).await?;
    if !exit_ok {
        bail!("Stage 1 subagent for '{name}' exited non-zero");
    }
    if !output_path.exists() {
        bail!(
            "Stage 1 subagent for '{name}' did not create {} — claude likely refused the Write \
             (check stderr above for permission/sandbox errors)",
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
        tree_sha: state.tree_sha.clone(),
        dirty_hash: state.dirty_hash.clone(),
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
    extra_writable_dir: &Path,
    timeout: Duration,
) -> Result<bool> {
    // `--setting-sources project,local` excludes the user's permission
    // allowlist; without an explicit `--allowed-tools` the Write needed
    // to deposit the surface YAML is silently denied. `--add-dir` grants
    // claude write access to the cache dir, which lives outside its cwd.
    let mut child = TokioCommand::new("claude")
        .arg("-p")
        .arg(prompt)
        .arg("--model")
        .arg(model)
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("project,local")
        .arg("--add-dir")
        .arg(extra_writable_dir)
        .arg("--allowed-tools")
        .arg("Read,Grep,Glob,Bash,Write,Task")
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

/// Render the catalog list for `{{CATALOG_PROJECTS}}` substitution as a
/// markdown bullet list excluding `current_project`. An empty result
/// (single project in catalog) emits a clear placeholder so the LLM
/// doesn't infer "no constraint".
fn render_catalog_for_prompt(current_project: &str, all_names: &[String]) -> String {
    let others: Vec<&String> = all_names.iter().filter(|n| *n != current_project).collect();
    if others.is_empty() {
        return "_(none — this project is the only catalog entry)_".to_string();
    }
    others
        .iter()
        .map(|n| format!("- {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render the current UTC time as an RFC-3339 string (`YYYY-MM-DDTHH:MM:SSZ`).
///
/// We roll our own rather than pull in `chrono` just for a timestamp —
/// the surface cache's `analysed_at` is informational, not parsed back.
pub(super) fn current_utc_rfc3339() -> String {
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

    use super::super::schema::{InteractionRoleHint, SurfaceRecord};
    use super::*;

    /// Content of the shipped `defaults/discover-stage1.md` embedded at
    /// compile time. Drift tests run off the same bytes the released
    /// binary copies to `<config-dir>/discover-stage1.md` on first run.
    const SHIPPED_STAGE1_PROMPT: &str = include_str!("../../defaults/discover-stage1.md");

    #[test]
    fn shipped_stage1_prompt_lists_every_interaction_role_hint() {
        // Bijection guard: every `InteractionRoleHint` variant must
        // appear as a vocabulary bullet of the form `- `\``<name>`\` — …`
        // in the Stage 1 prompt, and no other bullet of that shape may
        // exist (so a hint removed from the enum leaves no stale
        // vocabulary entry behind).
        use std::collections::BTreeSet;

        let enum_names: BTreeSet<String> = InteractionRoleHint::all()
            .iter()
            .map(|h| h.as_str().to_string())
            .collect();

        // The vocabulary section is the only place bullets of the form
        // `- `\``<kebab>`\` — …` appear in this prompt; scope the scan
        // to the "## Role hints (optional)" section to avoid picking up
        // unrelated backtick-wrapped bullets elsewhere.
        let section_start = SHIPPED_STAGE1_PROMPT
            .find("## Role hints (optional)")
            .expect("Stage 1 prompt must have a `## Role hints (optional)` section");
        let rest = &SHIPPED_STAGE1_PROMPT[section_start..];
        let section_end = rest[2..]
            .find("\n## ")
            .map(|i| i + 2)
            .unwrap_or(rest.len());
        let section = &rest[..section_end];

        let bullet = regex::Regex::new(r"(?m)^- `([a-z][a-z0-9-]*)`").unwrap();
        let rendered: BTreeSet<String> = bullet
            .captures_iter(section)
            .map(|c| c[1].to_string())
            .collect();

        let missing_from_prompt: Vec<_> = enum_names.difference(&rendered).cloned().collect();
        let missing_from_enum: Vec<_> = rendered.difference(&enum_names).cloned().collect();
        assert!(
            missing_from_prompt.is_empty(),
            "InteractionRoleHint variants missing from Stage 1 prompt vocabulary: {missing_from_prompt:?}"
        );
        assert!(
            missing_from_enum.is_empty(),
            "Stage 1 prompt lists vocabulary items not in InteractionRoleHint: {missing_from_enum:?}"
        );
    }

    #[test]
    fn shipped_stage1_prompt_declares_interaction_role_hints_field() {
        // The fields list must mention `interaction_role_hints` by name
        // and flag it as optional / closed-vocabulary, so Stage 1 knows
        // it may emit zero values without the subagent feeling obliged
        // to guess.
        assert!(
            SHIPPED_STAGE1_PROMPT.contains("`interaction_role_hints`"),
            "Stage 1 prompt must document the interaction_role_hints field by name"
        );
        assert!(
            SHIPPED_STAGE1_PROMPT.contains("optional, closed vocabulary"),
            "Stage 1 prompt must flag interaction_role_hints as optional + closed vocabulary"
        );
    }

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

        // Make a fresh git repo for the project, compute its real state.
        let project = tmp.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        init_repo_with_readme(&project);
        let state = compute_project_state(&project).unwrap();

        // Seed a cache entry with the exact tree_sha + dirty_hash.
        let file = SurfaceFile {
            schema_version: SURFACE_SCHEMA_VERSION,
            project: "Proj".to_string(),
            tree_sha: state.tree_sha.clone(),
            dirty_hash: state.dirty_hash.clone(),
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
            &[],
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        match outcome {
            Stage1Outcome::Cached(f) => {
                assert_eq!(f.tree_sha, state.tree_sha);
                assert_eq!(f.dirty_hash, state.dirty_hash);
                assert_eq!(f.surface.purpose, "cached");
            }
            other => panic!("expected Cached outcome, got {other:?}"),
        }
    }

    /// Seed the cache for a clean repo, then introduce an uncommitted
    /// change — the state's `dirty_hash` diverges so the cached entry
    /// must NOT be served.
    #[tokio::test]
    async fn cache_miss_when_tree_goes_dirty_after_seed() {
        let tmp = TempDir::new().unwrap();
        let cfg = tmp.path().join("cfg");
        std::fs::create_dir_all(&cfg).unwrap();

        let project = tmp.path().join("proj");
        std::fs::create_dir_all(&project).unwrap();
        init_repo_with_readme(&project);
        let clean_state = compute_project_state(&project).unwrap();

        // Seed cache as the clean state.
        cache::save_atomic(
            &cfg,
            &SurfaceFile {
                schema_version: SURFACE_SCHEMA_VERSION,
                project: "Proj".to_string(),
                tree_sha: clean_state.tree_sha.clone(),
                dirty_hash: clean_state.dirty_hash.clone(),
                analysed_at: "t".to_string(),
                surface: SurfaceRecord::default(),
            },
        )
        .unwrap();

        // Introduce a dirty edit — `dirty_hash` on current state now
        // differs from the cached entry, so we must NOT get a Cached
        // outcome. (We can't assert it *runs* claude in the unit test,
        // but we can assert no cache hit — in this test harness claude
        // is absent, so the fresh path will error out.)
        std::fs::write(project.join("README.md"), "edited\n").unwrap();
        let dirty_state = compute_project_state(&project).unwrap();
        assert_ne!(clean_state.dirty_hash, dirty_state.dirty_hash);

        let outcome = process_project(
            &cfg,
            "Proj",
            &project,
            "unused-model",
            "unused-prompt",
            &[],
            Duration::from_secs(1),
        )
        .await;

        // We expect a failure (no claude on PATH, short timeout) rather
        // than a successful Cached outcome — that's the cache-miss proof.
        assert!(
            outcome.is_err()
                || !matches!(
                    outcome.as_ref().unwrap(),
                    Stage1Outcome::Cached(_)
                ),
            "dirty tree must not serve stale cache"
        );
    }

    #[test]
    fn render_catalog_excludes_current_project_and_renders_bullets() {
        let names = vec!["Alpha".to_string(), "Beta".to_string(), "Gamma".to_string()];
        let rendered = render_catalog_for_prompt("Beta", &names);
        assert_eq!(rendered, "- Alpha\n- Gamma");
    }

    #[test]
    fn render_catalog_handles_solo_project() {
        let names = vec!["Solo".to_string()];
        let rendered = render_catalog_for_prompt("Solo", &names);
        assert!(
            rendered.starts_with("_(none"),
            "expected placeholder marker, got: {rendered}"
        );
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
