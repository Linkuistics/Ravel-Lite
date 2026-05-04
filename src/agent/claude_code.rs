use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::process::Command;

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;

use super::Agent;
use super::common::{
    STREAM_SNIPPET_BYTES, StreamLineOutcome, build_dispatch_plan_context, run_streaming_child,
    truncate_snippet,
};
use super::pty_capture;
use crate::config::load_tokens;
use crate::debug_log;
use crate::format::{
    FormattedOutput, ToolCall, clean_tool_name, extract_tool_detail, format_result_text,
    format_tool_call,
};
use crate::types::{AgentConfig, LlmPhase, PlanContext};
use crate::ui::UISender;

pub struct ClaudeCodeAgent {
    config: AgentConfig,
    config_root: String,
}

impl ClaudeCodeAgent {
    pub fn new(config: AgentConfig, config_root: String) -> Self {
        Self { config, config_root }
    }

    fn is_dangerous(&self, phase: &str) -> bool {
        self.config.params.get(phase)
            .and_then(|p| p.get("dangerous"))
            .and_then(|v| v.as_bool())
            == Some(true)
    }

    fn build_headless_args(
        &self,
        prompt: &str,
        phase: LlmPhase,
        ctx: &PlanContext,
    ) -> Result<Vec<String>> {
        let mut args = vec![
            "--strict-mcp-config".to_string(),
            "-p".to_string(),
            prompt.to_string(),
            "--verbose".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--add-dir".to_string(),
            ctx.plan_dir.clone(),
        ];

        for path in crate::state::targets::mounted_worktree_add_dirs(
            Path::new(&ctx.plan_dir),
            Path::new(&ctx.project_dir),
        )? {
            args.push("--add-dir".to_string());
            args.push(path);
        }

        if let Some(model) = self.config.models.get(phase.as_str()) {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }

        if self.is_dangerous(phase.as_str()) {
            args.push("--dangerously-skip-permissions".to_string());
        }

        if debug_log::is_enabled() {
            args.extend([
                "--debug-file".to_string(),
                debug_log::CLAUDE_DEBUG_FILE.to_string(),
            ]);
        }

        Ok(args)
    }
}

fn parse_stream_line(
    line: &str,
    phase: Option<LlmPhase>,
    shown_highlights: &mut HashSet<String>,
) -> StreamLineOutcome {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return StreamLineOutcome::Ignored;
    }

    let event: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => {
            return StreamLineOutcome::Malformed {
                snippet: truncate_snippet(trimmed, STREAM_SNIPPET_BYTES),
            };
        }
    };

    let Some(event_type) = event.get("type").and_then(|t| t.as_str()) else {
        return StreamLineOutcome::Ignored;
    };

    if event_type == "assistant" {
        if let Some(content) = event.get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                    continue;
                }
                let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let input = block.get("input").cloned().unwrap_or(serde_json::Value::Null);

                let tool = match name {
                    "Read" => ToolCall {
                        name: name.to_string(),
                        path: input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        detail: None,
                    },
                    "Write" | "Edit" => ToolCall {
                        name: name.to_string(),
                        path: input.get("file_path").and_then(|v| v.as_str()).map(|s| s.to_string()),
                        detail: None,
                    },
                    "Grep" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: Some(format!(
                            "\"{}\" in {}",
                            input.get("pattern").and_then(|v| v.as_str()).unwrap_or(""),
                            input.get("path").and_then(|v| v.as_str()).unwrap_or(".")
                        )),
                    },
                    "Glob" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: input.get("pattern").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    },
                    "Bash" => ToolCall {
                        name: name.to_string(),
                        path: None,
                        detail: input.get("command").and_then(|v| v.as_str()).map(|s| s.chars().take(120).collect()),
                    },
                    _ => ToolCall {
                        name: clean_tool_name(name),
                        path: None,
                        detail: Some(extract_tool_detail(&input)),
                    },
                };

                return StreamLineOutcome::Output(format_tool_call(&tool, phase, shown_highlights));
            }
        }
        return StreamLineOutcome::Ignored;
    }

    if event_type == "result" {
        if let Some(result_text) = event.get("result").and_then(|r| r.as_str()) {
            return StreamLineOutcome::Output(FormattedOutput {
                lines: format_result_text(result_text),
                persist: true,
            });
        }
    }

    StreamLineOutcome::Ignored
}

#[async_trait]
impl Agent for ClaudeCodeAgent {
    async fn invoke_interactive(
        &self,
        prompt: &str,
        ctx: &PlanContext,
    ) -> Result<()> {
        // `--output-format stream-json` only applies with `-p`/`--print`
        // (per `claude --help`). In interactive mode it puts claude into
        // a hybrid state where the TUI silently fails to render. Leave
        // interactive output to claude's default TUI.
        //
        // No `--add-dir <plan_dir>`: the plan dir is always a descendant
        // of `ctx.project_dir`, which is the cwd claude is launched in.
        // Claude already trusts its launch cwd, so the flag adds no
        // permission and on a fresh machine it can trigger an unseen
        // trust-grant modal that hangs the work phase after the banner.
        // The same rule guards `mounted_worktree_add_dirs`: it filters
        // out worktree paths that are descendants of cwd, emitting only
        // those that genuinely need the grant.
        let mut args: Vec<String> = Vec::new();

        for path in crate::state::targets::mounted_worktree_add_dirs(
            Path::new(&ctx.plan_dir),
            Path::new(&ctx.project_dir),
        )? {
            args.push("--add-dir".to_string());
            args.push(path);
        }

        if let Some(model) = self.config.models.get("work") {
            if !model.is_empty() {
                args.extend(["--model".to_string(), model.clone()]);
            }
        }

        if self.is_dangerous("work") {
            args.push("--dangerously-skip-permissions".to_string());
        }

        if debug_log::is_enabled() {
            args.extend([
                "--debug-file".to_string(),
                debug_log::CLAUDE_DEBUG_FILE.to_string(),
            ]);
        }

        args.push(prompt.to_string());

        if debug_log::is_enabled() {
            // PTY path: claude still owns a real tty (the slave), but
            // ravel-lite tees every byte through to the debug log. This
            // is what lets us diagnose "claude TUI invisible after the
            // banner" hangs from logs alone instead of needing the bug
            // to reproduce on a developer machine.
            return spawn_claude_via_pty(prompt, &args, ctx).await;
        }

        let status = std::process::Command::new("claude")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .context("Failed to spawn claude")
            .with_code(ErrorCode::IoError)?;

        debug_log::log(
            "claude exit (interactive, work)",
            &format!("status: {:?}", status.code()),
        );

        if !status.success() {
            bail_with!(
                ErrorCode::IoError,
                "claude exited with code {:?}",
                status.code()
            );
        }
        Ok(())
    }

    async fn invoke_headless(
        &self,
        prompt: &str,
        ctx: &PlanContext,
        phase: LlmPhase,
        agent_id: &str,
        tx: UISender,
    ) -> Result<()> {
        let args = self.build_headless_args(prompt, phase, ctx)?;

        if debug_log::is_enabled() {
            debug_log::log(
                &format!("claude spawn (headless, {})", phase.as_str()),
                &format!(
                    "cwd: {}\nagent_id: {}\n{}\nprompt:\n{}",
                    ctx.project_dir,
                    agent_id,
                    debug_log::format_argv("claude", &args),
                    indent_block(prompt),
                ),
            );
        }

        let child = Command::new("claude")
            .args(&args)
            .current_dir(&ctx.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn claude")
            .with_code(ErrorCode::IoError)?;

        run_streaming_child(child, phase, agent_id, "claude", tx, parse_stream_line).await
    }

    async fn dispatch_subagent(
        &self,
        prompt: &str,
        target_plan: &str,
        agent_id: &str,
        tx: UISender,
    ) -> Result<()> {
        let ctx = build_dispatch_plan_context(target_plan, self.config_root.clone())?;
        self.invoke_headless(prompt, &ctx, LlmPhase::Triage, agent_id, tx).await
    }

    fn tokens(&self) -> HashMap<String, String> {
        load_tokens(Path::new(&self.config_root), "claude-code")
            .unwrap_or_default()
    }
}

/// Indent every line of `body` by four spaces so it renders as a
/// nested block under a debug-log entry header.
fn indent_block(body: &str) -> String {
    body.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run the interactive work-phase claude under a PTY so every byte of
/// its TUI output is captured to the debug log. Used only when
/// `--debug` is on; the no-debug path keeps the inherit-stdio
/// behaviour (no perturbation for normal users).
async fn spawn_claude_via_pty(
    prompt: &str,
    args: &[String],
    ctx: &PlanContext,
) -> Result<()> {
    debug_log::log(
        "claude spawn (interactive, work)",
        &format!(
            "cwd: {}\nstdio: pty (full byte transcript follows)\n{}\nprompt:\n{}",
            ctx.project_dir,
            debug_log::format_argv("claude", args),
            indent_block(prompt),
        ),
    );

    let project_dir = ctx.project_dir.clone();
    let args = args.to_vec();
    let status = tokio::task::spawn_blocking(move || {
        pty_capture::run_pty_session("claude", &args, &project_dir, "claude")
    })
    .await
    .context("PTY task panicked")
    .with_code(ErrorCode::Internal)??;

    debug_log::log(
        "claude exit (interactive, work)",
        &format!("status: {status:?}"),
    );

    if !status.success() {
        bail_with!(ErrorCode::IoError, "claude exited with status {status:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(formatted: &FormattedOutput) -> String {
        formatted.lines.iter()
            .map(|l| l.0.iter().map(|s| s.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn expect_output(outcome: StreamLineOutcome) -> FormattedOutput {
        match outcome {
            StreamLineOutcome::Output(f) => f,
            StreamLineOutcome::Ignored => panic!("expected Output, got Ignored"),
            StreamLineOutcome::Malformed { snippet } => {
                panic!("expected Output, got Malformed({snippet})")
            }
        }
    }

    #[test]
    fn parse_tool_use_read() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/foo/bar.md"}}]}}"#;
        let mut shown = HashSet::new();
        let formatted = expect_output(parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown));
        assert!(!formatted.persist);
        let text = flat(&formatted);
        assert!(text.contains("Read"));
        assert!(text.contains("/foo/bar.md"));
    }

    #[test]
    fn parse_result_event() {
        let line = r#"{"type":"result","result":"[ADDED] New entry — description"}"#;
        let mut shown = HashSet::new();
        let formatted = expect_output(parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown));
        assert!(formatted.persist);
        assert!(flat(&formatted).contains("ADDED"));
    }

    #[test]
    fn parse_highlight_write_memory() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/plan/memory.yaml","content":"stuff"}}]}}"#;
        let mut shown = HashSet::new();
        assert!(expect_output(parse_stream_line(line, Some(LlmPhase::Reflect), &mut shown)).persist);
    }

    #[test]
    fn parse_ignores_empty_lines() {
        let mut shown = HashSet::new();
        assert!(matches!(
            parse_stream_line("", None, &mut shown),
            StreamLineOutcome::Ignored
        ));
        assert!(matches!(
            parse_stream_line("   ", None, &mut shown),
            StreamLineOutcome::Ignored
        ));
    }

    #[test]
    fn parse_unhandled_event_type_is_ignored() {
        // Valid JSON but nothing we display. Must NOT be classified as Malformed
        // — otherwise every system event would trigger a warning.
        let mut shown = HashSet::new();
        assert!(matches!(
            parse_stream_line(r#"{"type":"system","subtype":"init"}"#, None, &mut shown),
            StreamLineOutcome::Ignored
        ));
    }

    #[test]
    fn parse_malformed_json_surfaces_snippet() {
        // This is the scenario that used to silently disappear. The caller
        // now gets a snippet of the bad line so it can warn the user.
        let mut shown = HashSet::new();
        let outcome = parse_stream_line("this is not json", None, &mut shown);
        let StreamLineOutcome::Malformed { snippet } = outcome else {
            panic!("expected Malformed");
        };
        assert_eq!(snippet, "this is not json");
    }

    use crate::state::targets::{
        write_targets, Target, TargetsFile, TARGETS_SCHEMA_VERSION,
    };
    use tempfile::TempDir;

    fn agent_for_test() -> ClaudeCodeAgent {
        ClaudeCodeAgent::new(
            AgentConfig {
                models: HashMap::new(),
                thinking: HashMap::new(),
                params: HashMap::new(),
                provider: None,
            },
            "/unused".to_string(),
        )
    }

    fn ctx_for(plan_dir: &Path, project_dir: &Path) -> PlanContext {
        PlanContext {
            plan_dir: plan_dir.to_string_lossy().to_string(),
            project_dir: project_dir.to_string_lossy().to_string(),
            dev_root: "/unused".to_string(),
            related_plans: String::new(),
            config_root: "/unused".to_string(),
        }
    }

    fn write_one_target(plan_dir: &Path, working_root: &str) {
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![Target {
                repo_slug: "atlas".into(),
                component_id: "atlas-ontology".into(),
                working_root: working_root.into(),
                branch: "ravel-lite/sample/main".into(),
                path_segments: vec!["crates".into(), "atlas-ontology".into()],
            }],
        };
        write_targets(plan_dir, &targets).unwrap();
    }

    /// V1 layout: plan_dir is under project_dir; worktrees under plan_dir
    /// are reachable from cwd, so no extra `--add-dir` should appear.
    #[test]
    fn build_headless_args_skips_worktrees_inside_cwd() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path();
        let plan = project.join("LLM_STATE/sample");
        std::fs::create_dir_all(&plan).unwrap();
        write_one_target(&plan, ".worktrees/atlas");

        let agent = agent_for_test();
        let ctx = ctx_for(&plan, project);
        let args = agent.build_headless_args("p", LlmPhase::Work, &ctx).unwrap();

        let add_dir_count = args.iter().filter(|a| *a == "--add-dir").count();
        assert_eq!(
            add_dir_count, 1,
            "only the plan-dir --add-dir should be present; got: {args:?}"
        );
    }

    /// V2 layout: plan_dir lives outside project_dir; worktrees need
    /// explicit `--add-dir` to be reachable.
    #[test]
    fn build_headless_args_adds_worktrees_outside_cwd() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let plan = tmp.path().join("context/plans/sample");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&plan).unwrap();
        write_one_target(&plan, ".worktrees/atlas");

        let agent = agent_for_test();
        let ctx = ctx_for(&plan, &project);
        let args = agent.build_headless_args("p", LlmPhase::Work, &ctx).unwrap();

        let mounted_path = plan.join(".worktrees/atlas").to_string_lossy().to_string();
        let mut iter = args.iter();
        let mut found_mounted = false;
        while let Some(a) = iter.next() {
            if a == "--add-dir" {
                if let Some(next) = iter.clone().next() {
                    if next == &mounted_path {
                        found_mounted = true;
                        break;
                    }
                }
            }
        }
        assert!(
            found_mounted,
            "expected `--add-dir {mounted_path}` in argv: {args:?}"
        );
    }

    #[test]
    fn build_headless_args_emits_add_dir_per_target() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let plan = tmp.path().join("plan");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&plan).unwrap();

        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![
                Target {
                    repo_slug: "atlas".into(),
                    component_id: "a".into(),
                    working_root: ".worktrees/atlas".into(),
                    branch: "ravel-lite/p/main".into(),
                    path_segments: vec![],
                },
                Target {
                    repo_slug: "ravel".into(),
                    component_id: "b".into(),
                    working_root: ".worktrees/ravel".into(),
                    branch: "ravel-lite/p/main".into(),
                    path_segments: vec![],
                },
            ],
        };
        write_targets(&plan, &targets).unwrap();

        let agent = agent_for_test();
        let ctx = ctx_for(&plan, &project);
        let args = agent.build_headless_args("p", LlmPhase::Work, &ctx).unwrap();
        // One --add-dir for plan_dir + two for mounted worktrees.
        let add_dir_count = args.iter().filter(|a| *a == "--add-dir").count();
        assert_eq!(add_dir_count, 3, "argv: {args:?}");
    }
}
