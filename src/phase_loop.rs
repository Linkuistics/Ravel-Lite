use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::agent::Agent;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::backlog_transitions::backlog_transitions;
use crate::config_lua;
use crate::format::phase_info;
use crate::git::{
    apply_commits_spec, git_commit_plan, git_save_baseline, paths_changed_since_baseline,
    work_tree_snapshot, working_tree_status,
};
use crate::prompt::compose_prompt;
use crate::defeat_cascade::run_defeat_cascade;
use crate::state::filenames::PHASE_FILENAME;
use crate::state::focus_objections::delete_focus_objections;
use crate::state::intents::intents_path;
use crate::state::target_requests::drain_target_requests;
use crate::types::*;
use crate::ui::UI;

const HR: &str = "────────────────────────────────────────────────────";

fn read_phase(plan_dir: &Path) -> Result<Phase> {
    let content = fs::read_to_string(plan_dir.join(PHASE_FILENAME))
        .with_context(|| format!("Failed to read {PHASE_FILENAME}"))
        .with_code(ErrorCode::IoError)?;
    Phase::parse(content.trim())
        .with_context(|| format!("Unknown phase: {}", content.trim()))
        .with_code(ErrorCode::InvalidInput)
}

/// Writes the next phase marker to `phase.md`. Errors are propagated so
/// the loop doesn't silently advance past a filesystem failure (permissions,
/// full disk, stale handle) — the phase file is the single source of truth
/// for the loop's position, so a dropped write would re-invoke the agent on
/// the same phase and hide the real error.
fn write_phase(plan_dir: &Path, phase: Phase) -> Result<()> {
    let path = plan_dir.join(PHASE_FILENAME);
    fs::write(&path, phase.to_string())
        .with_context(|| format!("Failed to write phase marker: {}", path.display()))
        .with_code(ErrorCode::IoError)
}

fn plan_name(plan_dir: &Path) -> String {
    plan_dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Basename of the project directory, for the phase header. Many plans
/// share generic names like "core", so the project disambiguates which
/// session a banner belongs to in scrollback or when several sessions are up.
fn project_name(project_dir: &str) -> String {
    Path::new(project_dir)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Format the `project / plan` discriminator. Falls back to just the plan
/// when the project basename is empty (defensive — `project_dir` is normally
/// an absolute path under a real repo).
fn header_scope(project: &str, plan: &str) -> String {
    if project.is_empty() {
        plan.to_string()
    } else {
        format!("{project} / {plan}")
    }
}

fn log_phase_header(ui: &UI, phase: LlmPhase, project: &str, plan: &str) {
    let info = phase_info(phase);
    ui.log(&format!("\n{HR}"));
    ui.log(&format!("  ◆  {}  ·  {}", info.label, header_scope(project, plan)));
    ui.log(&format!("  {}", info.description));
    ui.log(HR);
}

fn log_commit(ui: &UI, phase_name: &str, plan: &str, result: &crate::git::CommitResult) {
    if result.committed {
        let first_line = result.message.lines().next().unwrap_or("");
        ui.log(&format!("\n  ⚙  COMMIT · {phase_name}  ·  {plan}  ·  {first_line}"));
    } else {
        ui.log(&format!("\n  ⚙  COMMIT · {phase_name}  ·  {plan}  ·  nothing to commit"));
    }
}

/// Maximum number of dirty paths to enumerate inline. The full list lives in
/// `git status` — the warning just needs enough context to alarm the user.
const DIRTY_PATH_DISPLAY_LIMIT: usize = 20;

/// Extract the path from a `git status --porcelain` line. Lines are
/// `XY path` (2 status chars + space + path); renames are `R  old -> new`
/// and the new path is returned. Returns `None` on unparseable input —
/// callers treat that as "preserve the entry" since a conservative keep
/// beats a silent drop when the narrowing filter can't classify a line.
fn parse_porcelain_path(line: &str) -> Option<&str> {
    let rest = line.get(3..)?;
    if let Some(arrow) = rest.find(" -> ") {
        Some(rest[arrow + 4..].trim())
    } else {
        Some(rest.trim())
    }
}

/// After the work-phase commit, the project tree should be clean: the agent
/// is expected to have committed every source-file edit it made during the
/// work phase, and `git_commit_plan` itself just committed the plan
/// bookkeeping. A non-empty `git status` here means the agent edited files
/// without committing them — the silent failure mode that has caused the
/// loop to advance past lost work as if the backlog were empty. Surface a
/// loud warning so the user can recover before phase state advances.
///
/// The dirty set is narrowed to paths the work agent could plausibly have
/// touched: untracked files (new since baseline by definition) plus any
/// tracked file that differs from the work baseline per `git diff
/// --name-only <baseline>`. This filters out sibling-plan in-flight writes
/// in multi-plan monorepos where `git status` is repo-wide. If the
/// baseline is missing or the diff call fails, the narrowing is skipped
/// and the original (over-inclusive) dirty list is used — strictly more
/// noisy, never less accurate.
///
/// Soft-failure: a transient git error here shouldn't kill the loop, so
/// the warning is best-effort. The status check itself is read-only.
fn warn_if_project_tree_dirty(ui: &UI, project_dir: &Path, plan_dir: &Path) {
    let dirty = match working_tree_status(project_dir) {
        Ok(d) => d,
        Err(_) => return,
    };
    if dirty.is_empty() {
        return;
    }

    let baseline_sha = fs::read_to_string(plan_dir.join("work-baseline"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let touched: Option<std::collections::HashSet<String>> = baseline_sha
        .as_deref()
        .and_then(|sha| paths_changed_since_baseline(project_dir, sha).ok());

    let filtered: Vec<&String> = dirty
        .iter()
        .filter(|line| {
            let Some(set) = &touched else { return true };
            if line.starts_with("??") {
                return true;
            }
            match parse_porcelain_path(line) {
                Some(path) => set.contains(path),
                None => true,
            }
        })
        .collect();

    if filtered.is_empty() {
        return;
    }

    ui.log("\n  ⚠  WARNING: uncommitted changes remain in the project tree");
    ui.log("     after the work commit. The work agent likely edited files");
    ui.log("     without committing them. Review and recover before continuing:");
    for line in filtered.iter().take(DIRTY_PATH_DISPLAY_LIMIT) {
        ui.log(&format!("       {line}"));
    }
    if filtered.len() > DIRTY_PATH_DISPLAY_LIMIT {
        ui.log(&format!(
            "       ... and {} more (run `git status` for the full list)",
            filtered.len() - DIRTY_PATH_DISPLAY_LIMIT
        ));
    }
}

/// Consume `focus-objections.yaml` at the close of triage.
///
/// The architecture frames this as "consume and remove"
/// (architecture-next.md §TRIAGE step 8): once the triage LLM has
/// read the prior cycle's objections and reflected them in its intent
/// / backlog edits, the file's purpose is spent, and leaving it on
/// disk would mislead the next triage into draining stale state.
///
/// Idempotent: returns `Ok(())` whether or not the file existed.
/// Wraps `delete_focus_objections` with phase-scoped error context so
/// a filesystem failure here surfaces clearly in the phase-loop log
/// rather than as a bare path-not-removed error.
fn consume_focus_objections(plan_dir: &Path) -> Result<()> {
    delete_focus_objections(plan_dir)
        .with_context(|| {
            format!(
                "Failed to consume focus-objections.yaml at {}",
                plan_dir.display()
            )
        })
        .with_code(ErrorCode::IoError)
}

/// Run the serves-intent defeat cascade if the plan has migrated to
/// the v2 wire shape (signal: `intents.yaml` exists).
///
/// Skipping when `intents.yaml` is absent is what keeps this safe to
/// wire in before the v1→v2 migrator ships: the dogfood plan and any
/// other v1 plan get no behaviour change, while v2 plans pick up the
/// architecture-next §Phase boundaries cascade. The gate is purely
/// presence-based — once the migrator runs, it produces both
/// `intents.yaml` and a v2 `backlog.yaml` atomically, so the cascade
/// finds a consistent pair to operate on.
///
/// Logged only when at least one item flipped — silent in the steady
/// state where most cycles produce no cascades.
fn cascade_defeated_items(plan_dir: &Path, ui: &UI, scope: &str) -> Result<()> {
    if !intents_path(plan_dir).exists() {
        return Ok(());
    }
    let cascaded = run_defeat_cascade(plan_dir)?;
    if !cascaded.is_empty() {
        ui.log(&format!(
            "\n  ⚙  CASCADE  ·  {scope}  ·  defeated {} backlog item(s) via serves-intent",
            cascaded.len()
        ));
    }
    Ok(())
}

/// Append the freshly-written `latest-session.yaml` record to
/// `session-log.yaml` so each cycle's narrative accumulates as a durable
/// audit trail.
///
/// `latest-session.yaml` is overwritten by analyse-work every cycle;
/// without this mirror write, prior sessions are lost. Runner-side on
/// purpose — mechanical file plumbing belongs here, not in a phase
/// prompt.
///
/// Idempotent on session id: if the log already contains a record whose
/// id matches `latest-session.yaml`'s id (e.g. a crash between this
/// call and `write_phase` forced a retry of `GitCommitWork`), the
/// second call is a no-op. This is strictly stronger than the earlier
/// tail-string check: a later manual edit to the log can't regress
/// the invariant.
///
/// Missing `latest-session.yaml` is also a no-op — the first work
/// cycle of a fresh plan has no session record to propagate. Analyse-
/// work is expected to produce the file on every real cycle.
fn append_session_log(plan_dir: &Path) -> Result<()> {
    crate::state::session_log::append_latest_to_log(plan_dir)
        .with_context(|| {
            format!(
                "Failed to append latest-session.yaml to session-log.yaml at {}",
                plan_dir.display()
            )
        })
        .with_code(ErrorCode::IoError)?;
    Ok(())
}

async fn handle_script_phase(
    phase: ScriptPhase,
    plan_dir: &Path,
    project_dir: &Path,
    ui: &UI,
) -> Result<bool> {
    let name = plan_name(plan_dir);
    let project = project_name(&project_dir.to_string_lossy());
    let scope = header_scope(&project, &name);

    // Invariant: each script-phase handler advances `phase.md` BEFORE
    // calling `git_commit_plan`, so the phase transition is captured in
    // the same commit as that phase's other plan-state writes. Order
    // matters — writing after the commit would leave `phase.md` dirty at
    // the user-prompt points, which leaks into sibling plans in
    // multi-plan monorepos (where `warn_if_project_tree_dirty` scans the
    // whole project dir and mistakes the leak for work the agent forgot
    // to commit).
    //
    // The cycle order is `triage → work → analyse-work → reflect`,
    // committed at four boundaries. The baseline filename written by
    // each script phase is named after the *LLM phase that consumes it*:
    // `git-commit-triage` writes `work-baseline`, `git-commit-work`
    // writes `analyse-work-baseline`, etc. See `docs/architecture-next.md`
    // §Phase boundaries.
    match phase {
        ScriptPhase::GitCommitTriage => {
            // Triage opens the cycle. Commit triage's plan-state mutations
            // (intents, backlog, memory hygiene) and capture the post-
            // triage SHA as `work-baseline` so the next analyse-work can
            // diff `{{BACKLOG_TRANSITIONS}}` against the start of work.
            //
            // Drain `focus-objections.yaml` before writing the phase
            // marker so the deletion is captured in the same commit as
            // triage's other plan-state edits — the file's purpose is
            // spent once triage has read it (architecture-next.md
            // §TRIAGE step 8 "Consume and remove focus-objections.yaml").
            consume_focus_objections(plan_dir)?;

            // Run the serves-intent defeat cascade if the plan has a
            // v2 `intents.yaml`. v1 plans (no intents.yaml) skip
            // cleanly; v2 plans pick up cascade-induced backlog flips
            // in this same triage commit so downstream phases see a
            // consistent backlog. See architecture-next.md §TRIAGE
            // step 2 ("Mechanically propagate intent status changes
            // through serves-intent edges to backlog items. Run by
            // the runner, not the LLM.")
            cascade_defeated_items(plan_dir, ui, &scope)?;

            write_phase(plan_dir, Phase::Llm(LlmPhase::Work))?;
            let result = git_commit_plan(plan_dir, &name, "triage")?;
            log_commit(ui, "triage", &scope, &result);

            git_save_baseline(plan_dir, "work-baseline");
            let baseline_result = git_commit_plan(plan_dir, &name, "save-work-baseline")?;
            log_commit(ui, "save-work-baseline", &scope, &baseline_result);
            Ok(true)
        }
        ScriptPhase::GitCommitWork => {
            // Sits between work and analyse-work. Commits any plan-state
            // changes the work agent made in-flight (memory entries,
            // focus-objections.yaml). Source-tree changes from work
            // remain dirty for analyse-work to inspect via the
            // work-tree snapshot.
            write_phase(plan_dir, Phase::Llm(LlmPhase::AnalyseWork))?;
            let result = git_commit_plan(plan_dir, &name, "work")?;
            log_commit(ui, "work", &scope, &result);

            git_save_baseline(plan_dir, "analyse-work-baseline");
            let baseline_result =
                git_commit_plan(plan_dir, &name, "save-analyse-work-baseline")?;
            log_commit(ui, "save-analyse-work-baseline", &scope, &baseline_result);
            Ok(true)
        }
        ScriptPhase::GitCommitAnalyseWork => {
            append_session_log(plan_dir)?;
            write_phase(plan_dir, Phase::Llm(LlmPhase::Reflect))?;
            // Apply analyse-work's `commits.yaml` to commit the source-
            // tree changes work made (partitioned per the spec, falling
            // back to a single catch-all when the spec is missing).
            // Then commit any plan-state changes analyse-work itself
            // produced (latest-session.yaml, backlog status repairs).
            let results = apply_commits_spec(project_dir, plan_dir, &name, "analyse-work")?;
            for result in &results {
                log_commit(ui, "analyse-work", &scope, result);
            }

            git_save_baseline(plan_dir, "reflect-baseline");
            let baseline_result = git_commit_plan(plan_dir, &name, "save-reflect-baseline")?;
            log_commit(ui, "save-reflect-baseline", &scope, &baseline_result);

            warn_if_project_tree_dirty(ui, project_dir, plan_dir);
            Ok(true)
        }
        ScriptPhase::GitCommitReflect => {
            // Closes the cycle. Commits reflect's memory mutations,
            // captures the post-reflect SHA as `triage-baseline` so the
            // next cycle's triage knows what changed, and writes the
            // cycle-opening phase marker so a fresh `ravel-lite run`
            // picks up at triage.
            write_phase(plan_dir, Phase::Llm(LlmPhase::Triage))?;
            let result = git_commit_plan(plan_dir, &name, "reflect")?;
            log_commit(ui, "reflect", &scope, &result);

            git_save_baseline(plan_dir, "triage-baseline");
            let baseline_result = git_commit_plan(plan_dir, &name, "save-triage-baseline")?;
            log_commit(ui, "save-triage-baseline", &scope, &baseline_result);

            // Exit phase_loop after one full cycle. The outer loop
            // (run_single_plan / multi-plan dispatcher) decides whether
            // to start another cycle.
            Ok(false)
        }
    }
}

pub async fn phase_loop(
    agent: Arc<dyn Agent>,
    ctx: &PlanContext,
    ui: &UI,
) -> Result<()> {
    let tokens = agent.tokens();
    let plan_dir = Path::new(&ctx.plan_dir);
    let project_dir = Path::new(&ctx.project_dir);
    let config_root = Path::new(&ctx.config_root);
    let name = plan_name(plan_dir);
    let project = project_name(&ctx.project_dir);

    // Resolve `<global>/config.lua` + `<plan>/config.lua` once per
    // loop so every phase composition can pull plan-level
    // `ravel.append_prompt` registrations without re-parsing Lua. The
    // shared/agents/tokens slices of the resolved config are already
    // surfaced through `agent.tokens()` and the global loaders, so
    // here we just keep the appends.
    let resolved = config_lua::resolve(config_root, Some(plan_dir))?;

    if let Err(e) = agent.setup(ctx).await {
        ui.log(&format!("  ✗  Setup failed: {e}"));
    }

    loop {
        // Phase-boundary mechanics, per `docs/architecture-next.md`
        // §Phase boundaries: drain `target-requests.yaml` before
        // reading the next phase, so any newly requested mounts are
        // visible to the next prompt. Empty/absent queue is the common
        // case and the drain is a no-op then.
        let mounted = drain_target_requests(plan_dir, config_root)?;
        if mounted > 0 {
            ui.log(&format!(
                "\n  ⚙  MOUNT  ·  {}  ·  drained {mounted} request(s) from target-requests.yaml",
                header_scope(&project, &name)
            ));
        }

        let phase = read_phase(plan_dir)?;

        match phase {
            Phase::Script(sp) => {
                if !handle_script_phase(sp, plan_dir, project_dir, ui).await? {
                    // Script-phase handler signalled end-of-cycle.
                    // `GitCommitReflect` closes the cycle in the new
                    // triage-first ordering; callers decide what happens
                    // next.
                    return Ok(());
                }
                continue;
            }
            Phase::Llm(lp) => {
                let agent_id = "main";
                crate::term_title::set_title(&project, &name, lp.as_str());
                log_phase_header(ui, lp, &project, &name);

                // First-run fallback: in steady state, `git-commit-triage`
                // prepares `work-baseline` as part of its atomic commit.
                // On a brand-new plan that starts at `work` with no prior
                // triage, `work-baseline` doesn't exist yet — seed it so
                // analyse-work has a baseline SHA to diff against.
                if lp == LlmPhase::Work && !plan_dir.join("work-baseline").exists() {
                    git_save_baseline(plan_dir, "work-baseline");
                }

                // analyse-work needs a live snapshot of the work tree so the
                // prompt can (a) show the LLM exactly what changed since the
                // baseline and (b) force it to commit or justify every path.
                // Captured at prompt-compose time so any hand-edits the user
                // made between work exit and analyse-work start are included.
                let prompt = if lp == LlmPhase::AnalyseWork {
                    let mut augmented = tokens.clone();
                    let baseline_sha = fs::read_to_string(plan_dir.join("work-baseline"))
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let snapshot = if baseline_sha.is_empty() {
                        "(work-baseline missing; no snapshot available)".to_string()
                    } else {
                        work_tree_snapshot(project_dir, &baseline_sha)
                    };
                    augmented.insert("WORK_TREE_STATUS".to_string(), snapshot);
                    // Backlog delta since baseline: status flips, results
                    // additions, added/deleted tasks. Computed here rather
                    // than asked of the LLM because it's a pure YAML diff —
                    // the "Never do in an LLM what you can do in code" rule.
                    let transitions = backlog_transitions(plan_dir, &baseline_sha);
                    augmented.insert("BACKLOG_TRANSITIONS".to_string(), transitions);
                    compose_prompt(lp, ctx, &augmented, resolved.appends_for(lp.as_str()))?
                } else {
                    compose_prompt(lp, ctx, &tokens, resolved.appends_for(lp.as_str()))?
                };
                let tx = ui.sender();

                ui.register_agent(agent_id);

                if lp == LlmPhase::Work {
                    ui.suspend().await;
                    agent.invoke_interactive(&prompt, ctx).await?;
                    ui.resume();
                } else {
                    agent.invoke_headless(&prompt, ctx, lp, agent_id, tx).await?;
                }

                let new_phase = read_phase(plan_dir)?;
                if new_phase == phase {
                    ui.log(&format!("\n  ✗  Phase did not advance from {phase}. Stopping."));
                    return Ok(());
                }
            }
        }
    }
}

/// Top-level entry point for single-plan `ravel-lite run`. Repeatedly
/// invokes `phase_loop` (which now exits after one full cycle), asking
/// the user between cycles whether to continue. The prompt used to live
/// inside `handle_script_phase(GitCommitTriage)`; it moved out here so
/// multi-plan mode can run one cycle and return to its own survey-based
/// selection loop without a spurious confirm in between.
pub async fn run_single_plan(
    agent: Arc<dyn Agent>,
    ctx: PlanContext,
    ui: &UI,
) -> Result<()> {
    loop {
        phase_loop(agent.clone(), &ctx, ui).await?;
        if !ui.confirm("Proceed to next work phase?").await {
            ui.log("\nExiting.");
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_strips_to_basename() {
        assert_eq!(project_name("/Users/x/Development/ravel-lite"), "ravel-lite");
        assert_eq!(project_name("ravel-lite"), "ravel-lite");
    }

    #[test]
    fn project_name_handles_trailing_slash() {
        // Path::file_name returns None for paths ending in `..` but a real
        // trailing slash collapses to the directory basename.
        assert_eq!(project_name("/Users/x/ravel-lite/"), "ravel-lite");
    }

    #[test]
    fn project_name_empty_when_unparseable() {
        assert_eq!(project_name(""), "");
        assert_eq!(project_name("/"), "");
    }

    #[test]
    fn header_scope_combines_project_and_plan() {
        assert_eq!(header_scope("ravel-lite", "core"), "ravel-lite / core");
    }

    #[test]
    fn header_scope_falls_back_to_plan_when_project_empty() {
        // Defensive: if project_dir somehow resolves to "" the banner still
        // identifies the plan rather than rendering "  / core" with a dangling slash.
        assert_eq!(header_scope("", "core"), "core");
    }

    #[test]
    fn parse_porcelain_path_handles_modified_file() {
        assert_eq!(parse_porcelain_path(" M src/foo.rs"), Some("src/foo.rs"));
        assert_eq!(parse_porcelain_path("M  src/foo.rs"), Some("src/foo.rs"));
    }

    #[test]
    fn parse_porcelain_path_handles_untracked() {
        assert_eq!(parse_porcelain_path("?? new.rs"), Some("new.rs"));
    }

    #[test]
    fn parse_porcelain_path_returns_new_name_for_rename() {
        assert_eq!(
            parse_porcelain_path("R  old/path.rs -> new/path.rs"),
            Some("new/path.rs")
        );
    }

    #[test]
    fn parse_porcelain_path_returns_none_on_too_short_input() {
        assert_eq!(parse_porcelain_path(""), None);
        assert_eq!(parse_porcelain_path("ab"), None);
    }

    #[test]
    fn write_phase_writes_marker_file() {
        let dir = tempfile::TempDir::new().unwrap();
        write_phase(dir.path(), Phase::Llm(LlmPhase::Reflect)).unwrap();
        let contents = fs::read_to_string(dir.path().join(PHASE_FILENAME)).unwrap();
        assert_eq!(contents, "reflect");
    }

    #[test]
    fn consume_focus_objections_removes_existing_file() {
        // Triage-entry drain contract: after the triage LLM exits, a
        // present `focus-objections.yaml` must be removed so the next
        // triage doesn't re-read stale objections.
        use crate::state::focus_objections::{
            focus_objections_path, write_focus_objections, FocusObjectionsFile,
        };
        let tmp = tempfile::TempDir::new().unwrap();
        write_focus_objections(tmp.path(), &FocusObjectionsFile::default()).unwrap();
        assert!(focus_objections_path(tmp.path()).exists());

        consume_focus_objections(tmp.path()).unwrap();

        assert!(
            !focus_objections_path(tmp.path()).exists(),
            "drain must remove the file"
        );
    }

    #[test]
    fn consume_focus_objections_is_idempotent_when_file_absent() {
        // Steady-state contract: the common case is "no objections
        // raised", so the drain runs every triage close and must be a
        // no-op when there is nothing to remove.
        let tmp = tempfile::TempDir::new().unwrap();
        consume_focus_objections(tmp.path()).unwrap();
        consume_focus_objections(tmp.path()).unwrap();
    }

    #[test]
    fn cascade_defeated_items_skips_when_intents_yaml_is_absent() {
        // v1-plan compatibility gate: the dogfood plan has no
        // intents.yaml today, and the LLM_STATE freeze blocks the
        // v1→v2 migrator from shipping yet. The wiring must be a
        // pure no-op in that state — neither erroring nor reading
        // backlog.yaml (which would fail on v1 wire shape).
        let tmp = tempfile::TempDir::new().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let ui = UI::new(tx);
        cascade_defeated_items(tmp.path(), &ui, "test/scope").unwrap();
    }

    #[test]
    fn write_phase_errors_when_directory_is_missing() {
        // Guard: fs::write previously returned silently via `let _ = ...`, so
        // a missing plan dir would advance the loop with stale phase state.
        // The new signature surfaces the error with the target path.
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        let err = write_phase(&missing, Phase::Llm(LlmPhase::Work))
            .expect_err("write should fail on a missing directory");
        assert!(
            err.to_string().contains(PHASE_FILENAME),
            "error should name the target file: {err}"
        );
    }
}
