// src/survey.rs
//
// Multi-project plan status survey. Gathers `phase.md`, `backlog.md`,
// and `memory.md` from every plan directory under one or more roots,
// renders them as a single prompt, and shells out to a headless
// `claude` session for LLM-driven summarisation and prioritisation.
//
// The command is intentionally single-shot and read-only: no tool use,
// no file writes, no session persistence. Fresh context per invocation
// by construction.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use crate::config::{load_agent_config, load_shared_config};

/// Fallback model when neither `--model` nor `models.survey` is
/// configured. A cheap, fast model is appropriate: survey is a
/// summarisation task over plain-text inputs.
pub const DEFAULT_SURVEY_MODEL: &str = "claude-haiku-4-5";

/// Relative path to the survey prompt template inside a config dir.
pub const SURVEY_PROMPT_PATH: &str = "survey.md";

/// A single plan's state, bundled for inclusion in the survey prompt.
#[derive(Debug)]
pub struct PlanSnapshot {
    pub project: String,
    pub plan: String,
    pub phase: String,
    pub backlog: Option<String>,
    pub memory: Option<String>,
}

/// Walk `root` looking for plan directories. A directory is a plan iff
/// it contains a `phase.md` file; this matches the convention used
/// everywhere else in Raveloop. Returned plans are sorted by plan name
/// for deterministic output.
pub fn discover_plans(root: &Path) -> Result<Vec<PlanSnapshot>> {
    let project = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("(unknown)")
        .to_string();

    let mut plans = Vec::new();

    let entries = fs::read_dir(root)
        .with_context(|| format!("Failed to read plan root {}", root.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let phase_file = path.join("phase.md");
        if !phase_file.exists() {
            continue;
        }

        let plan = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("(unnamed)")
            .to_string();
        let phase = fs::read_to_string(&phase_file)
            .with_context(|| format!("Failed to read {}", phase_file.display()))?
            .trim()
            .to_string();
        let backlog = fs::read_to_string(path.join("backlog.md")).ok();
        let memory = fs::read_to_string(path.join("memory.md")).ok();

        plans.push(PlanSnapshot {
            project: project.clone(),
            plan,
            phase,
            backlog,
            memory,
        });
    }

    plans.sort_by(|a, b| a.plan.cmp(&b.plan));
    Ok(plans)
}

/// Render all discovered plans as a single Markdown block to append
/// after the survey prompt. Missing backlog/memory files are noted
/// explicitly rather than silently elided, so the LLM can distinguish
/// "empty" from "absent".
pub fn render_survey_input(plans: &[PlanSnapshot]) -> String {
    let mut out = String::new();
    for plan in plans {
        out.push_str(&format!(
            "\n## Plan: {}/{}\n\n### phase\n{}\n\n",
            plan.project, plan.plan, plan.phase
        ));
        match &plan.backlog {
            Some(b) => out.push_str(&format!("### backlog.md\n{b}\n\n")),
            None => out.push_str("### backlog.md\n(missing)\n\n"),
        }
        match &plan.memory {
            Some(m) => out.push_str(&format!("### memory.md\n{m}\n\n")),
            None => out.push_str("### memory.md\n(missing)\n\n"),
        }
        out.push_str("---\n");
    }
    out
}

/// Read the survey prompt template from `<config_root>/survey.md`.
pub fn load_survey_prompt(config_root: &Path) -> Result<String> {
    let path = config_root.join(SURVEY_PROMPT_PATH);
    fs::read_to_string(&path)
        .with_context(|| format!("Failed to read survey prompt at {}", path.display()))
}

/// Resolve which model to use for the survey call. Precedence:
///   1. explicit `--model` flag on the CLI
///   2. `models.survey` in the agent's config
///   3. `DEFAULT_SURVEY_MODEL` constant
fn resolve_model(
    agent_config: &crate::types::AgentConfig,
    flag_override: Option<String>,
) -> String {
    flag_override
        .or_else(|| agent_config.models.get("survey").cloned())
        .unwrap_or_else(|| DEFAULT_SURVEY_MODEL.to_string())
}

/// End-to-end survey runner. Gathers plans across every `--root`,
/// composes the prompt, invokes the `claude` CLI headlessly, and
/// prints the LLM's response to stdout.
pub async fn run_survey(
    config_root: &Path,
    roots: &[PathBuf],
    model_override: Option<String>,
) -> Result<()> {
    let shared = load_shared_config(config_root)?;
    if shared.agent != "claude-code" {
        anyhow::bail!(
            "survey currently only supports agent 'claude-code' (configured agent: '{}').",
            shared.agent
        );
    }

    let agent_config = load_agent_config(config_root, &shared.agent)?;
    let model = resolve_model(&agent_config, model_override);

    let mut all_plans = Vec::new();
    for root in roots {
        if !root.is_dir() {
            anyhow::bail!(
                "Plan root {} does not exist or is not a directory.",
                root.display()
            );
        }
        let plans = discover_plans(root)?;
        if plans.is_empty() {
            eprintln!(
                "warning: plan root {} contained no plan directories (no phase.md found)",
                root.display()
            );
        }
        all_plans.extend(plans);
    }
    if all_plans.is_empty() {
        anyhow::bail!("No plans discovered in any of the supplied --root directories.");
    }

    let survey_prompt = load_survey_prompt(config_root)?;
    let plan_input = render_survey_input(&all_plans);
    let full_prompt = format!("{survey_prompt}\n\n---\n{plan_input}");

    eprintln!(
        "Surveying {} plan(s) across {} root(s) using model {}...",
        all_plans.len(),
        roots.len(),
        model
    );

    let mut child = TokioCommand::new("claude")
        .arg("-p")
        .arg(&full_prompt)
        .arg("--model")
        .arg(&model)
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("project,local")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to spawn 'claude' CLI. Ensure it is installed and on PATH.")?;

    let mut stdout = child
        .stdout
        .take()
        .context("claude CLI stdout pipe was unexpectedly unavailable")?;
    let mut output = String::new();
    stdout.read_to_string(&mut output).await?;

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("claude CLI exited with status {status}");
    }

    print!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    use crate::types::AgentConfig;

    fn write_plan(root: &Path, name: &str, phase: &str, backlog: Option<&str>, memory: Option<&str>) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("phase.md"), phase).unwrap();
        if let Some(b) = backlog {
            fs::write(dir.join("backlog.md"), b).unwrap();
        }
        if let Some(m) = memory {
            fs::write(dir.join("memory.md"), m).unwrap();
        }
    }

    fn empty_agent_config(models: &[(&str, &str)]) -> AgentConfig {
        let mut m = HashMap::new();
        for (k, v) in models {
            m.insert(k.to_string(), v.to_string());
        }
        AgentConfig {
            models: m,
            thinking: HashMap::new(),
            params: HashMap::new(),
            provider: None,
        }
    }

    #[test]
    fn discover_plans_finds_directories_with_phase_md() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_plan(root, "plan-a", "work\n", Some("# backlog a\n"), Some("# memory a\n"));
        write_plan(root, "plan-b", "triage\n", Some("# backlog b\n"), None);
        // A directory WITHOUT phase.md is ignored.
        fs::create_dir_all(root.join("not-a-plan")).unwrap();
        fs::write(root.join("not-a-plan").join("backlog.md"), "noise").unwrap();

        let plans = discover_plans(root).unwrap();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].plan, "plan-a");
        assert_eq!(plans[1].plan, "plan-b");
    }

    #[test]
    fn discover_plans_uses_root_basename_as_project() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("Mnemosyne-LLM_STATE");
        fs::create_dir_all(&root).unwrap();
        write_plan(&root, "plan-x", "work\n", None, None);

        let plans = discover_plans(&root).unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].project, "Mnemosyne-LLM_STATE");
    }

    #[test]
    fn discover_plans_trims_phase_whitespace() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_plan(root, "plan-a", "  \n work \n\n", None, None);

        let plans = discover_plans(root).unwrap();
        assert_eq!(plans[0].phase, "work");
    }

    #[test]
    fn discover_plans_records_missing_backlog_and_memory_as_none() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_plan(root, "plan-a", "work\n", None, None);

        let plans = discover_plans(root).unwrap();
        assert!(plans[0].backlog.is_none());
        assert!(plans[0].memory.is_none());
    }

    #[test]
    fn discover_plans_returns_sorted_by_plan_name() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_plan(root, "zeta", "work\n", None, None);
        write_plan(root, "alpha", "work\n", None, None);
        write_plan(root, "mu", "work\n", None, None);

        let plans = discover_plans(root).unwrap();
        let names: Vec<_> = plans.iter().map(|p| p.plan.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn discover_plans_errors_when_root_unreadable() {
        let missing = PathBuf::from("/definitely/not/a/path/for/survey/test");
        assert!(discover_plans(&missing).is_err());
    }

    #[test]
    fn render_survey_input_includes_project_and_plan_names() {
        let plans = vec![PlanSnapshot {
            project: "Mnemosyne".into(),
            plan: "sub-A".into(),
            phase: "work".into(),
            backlog: Some("# backlog".into()),
            memory: Some("# memory".into()),
        }];
        let out = render_survey_input(&plans);
        assert!(out.contains("## Plan: Mnemosyne/sub-A"));
        assert!(out.contains("### phase\nwork"));
        assert!(out.contains("### backlog.md\n# backlog"));
        assert!(out.contains("### memory.md\n# memory"));
    }

    #[test]
    fn render_survey_input_marks_missing_files_explicitly() {
        let plans = vec![PlanSnapshot {
            project: "P".into(),
            plan: "x".into(),
            phase: "work".into(),
            backlog: None,
            memory: None,
        }];
        let out = render_survey_input(&plans);
        assert!(out.contains("### backlog.md\n(missing)"));
        assert!(out.contains("### memory.md\n(missing)"));
    }

    #[test]
    fn render_survey_input_separates_plans_with_horizontal_rule() {
        let plans = vec![
            PlanSnapshot {
                project: "P".into(),
                plan: "a".into(),
                phase: "work".into(),
                backlog: None,
                memory: None,
            },
            PlanSnapshot {
                project: "P".into(),
                plan: "b".into(),
                phase: "triage".into(),
                backlog: None,
                memory: None,
            },
        ];
        let out = render_survey_input(&plans);
        assert_eq!(out.matches("\n---\n").count(), 2);
    }

    #[test]
    fn load_survey_prompt_reads_from_config_root() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("survey.md"), "hello prompt").unwrap();
        assert_eq!(load_survey_prompt(tmp.path()).unwrap(), "hello prompt");
    }

    #[test]
    fn load_survey_prompt_errors_when_missing() {
        let tmp = TempDir::new().unwrap();
        let err = load_survey_prompt(tmp.path()).unwrap_err();
        assert!(format!("{err:#}").contains("survey.md"));
    }

    #[test]
    fn resolve_model_prefers_cli_flag() {
        let cfg = empty_agent_config(&[("survey", "configured-model")]);
        let resolved = resolve_model(&cfg, Some("flag-model".into()));
        assert_eq!(resolved, "flag-model");
    }

    #[test]
    fn resolve_model_falls_back_to_agent_config_survey_key() {
        let cfg = empty_agent_config(&[("survey", "configured-model")]);
        let resolved = resolve_model(&cfg, None);
        assert_eq!(resolved, "configured-model");
    }

    #[test]
    fn resolve_model_uses_default_when_nothing_configured() {
        let cfg = empty_agent_config(&[]);
        let resolved = resolve_model(&cfg, None);
        assert_eq!(resolved, DEFAULT_SURVEY_MODEL);
    }
}
