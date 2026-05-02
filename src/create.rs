// src/create.rs
//
// Interactive plan-creation subcommand. Spawns a headful `claude`
// session that reads the `create-plan.md` prompt template from the
// user's config directory, appends the target plan path, and inherits
// the parent's stdio so the user drives the conversation directly.
//
// Unlike `survey` (read-only one-shot), `create` writes files — so
// it needs the agent's interactive REPL and tool-approval flow. The
// Ravel-Lite process is a thin wrapper: path validation, prompt
// composition, subprocess spawn with inherited stdio, post-hoc
// verification.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::Command as TokioCommand;

use crate::component_ref::ComponentRef;
use crate::config::{load_agent_config, load_shared_config};
use crate::init::require_embedded;
use crate::prompt::substitute_tokens;
use crate::state::filenames::{
    BACKLOG_FILENAME, DREAM_WORD_COUNT_FILENAME, INTENTS_FILENAME, MEMORY_FILENAME, PHASE_FILENAME,
};
use crate::state::target_requests::{
    write_target_requests, TargetRequest, TargetRequestsFile, TARGET_REQUESTS_SCHEMA_VERSION,
};
use crate::types::PlanContext;

/// Relative path to the create-plan prompt template inside a config dir.
pub const CREATE_PLAN_PROMPT_PATH: &str = "create-plan.md";

/// Tools pre-approved for the create session. Restricting Bash to
/// specific ravel-lite invocations prevents accidental shell-out during
/// the dialogue while keeping the catalog and state CLIs friction-free.
const CREATE_ALLOWED_TOOLS: &str = "Bash(ravel-lite atlas:*),\
Bash(ravel-lite repo:*),\
Bash(ravel-lite state intents:*),\
Bash(ravel-lite state backlog:*),\
Bash(ravel-lite state memory:*),\
Bash(ravel-lite state target-requests:*),\
Read,Write,Glob,Grep";

/// Compose the v2 create prompt. Substitutes `{{PLAN}}` (the absolute
/// plan-dir path) via the canonical `substitute_tokens` path, which
/// hard-errors on any leftover `{{NAME}}` placeholders. Other tokens
/// (`{{PROJECT}}`, `{{ORCHESTRATOR}}`) substitute to `context_root`
/// because v2 plans have no per-project root — the context is the
/// closest analogue. `{{DEV_ROOT}}` and `{{RELATED_PLANS}}` substitute
/// to empty strings; the v2 template should not reference them.
pub fn compose_create_prompt(
    template: &str,
    abs_plan_dir: &Path,
    context_root: &Path,
) -> Result<String> {
    let ctx = PlanContext {
        dev_root: String::new(),
        project_dir: context_root.display().to_string(),
        plan_dir: abs_plan_dir.display().to_string(),
        config_root: context_root.display().to_string(),
        related_plans: String::new(),
    };
    let tokens: HashMap<String, String> = HashMap::new();
    substitute_tokens(template, &ctx, &tokens)
}

/// Validate a v2 plan name. Plan names appear in git ref components
/// (`ravel-lite/<plan>/main`), commit messages, survey output, and
/// `targets.yaml`, so the rules match git ref-component validity plus
/// extra footgun-avoidance.
pub fn validate_plan_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Plan name cannot be empty.");
    }
    if name == "." || name == ".." {
        anyhow::bail!("Plan name cannot be `.` or `..`.");
    }
    if name.contains("..") {
        anyhow::bail!("Plan name {name:?} contains `..`.");
    }
    if name.starts_with('.') {
        anyhow::bail!("Plan name {name:?} cannot start with `.`.");
    }
    if name.starts_with('-') {
        anyhow::bail!("Plan name {name:?} cannot start with `-`.");
    }
    if name.ends_with(".lock") {
        anyhow::bail!("Plan name {name:?} cannot end with `.lock`.");
    }
    for c in name.chars() {
        if c.is_whitespace() {
            anyhow::bail!("Plan name {name:?} contains whitespace.");
        }
        if c.is_control() {
            anyhow::bail!("Plan name {name:?} contains control characters.");
        }
        if matches!(c, '/' | '\\' | '~' | '^' | ':' | '?' | '*' | '[') {
            anyhow::bail!("Plan name {name:?} contains invalid character {c:?}.");
        }
    }
    Ok(())
}

/// Resolve a v2 plan name to an absolute directory under
/// `<context_root>/plans/<plan_name>/`. Validates the name first and
/// errors if the resolved path already exists. Does not create
/// directories — caller is `scaffold_plan_dir`.
pub fn resolve_plan_dir(context_root: &Path, plan_name: &str) -> Result<PathBuf> {
    validate_plan_name(plan_name)?;
    let plan_dir = context_root.join("plans").join(plan_name);
    if plan_dir.exists() {
        anyhow::bail!(
            "Plan directory {} already exists. create will not overwrite an existing plan.",
            plan_dir.display()
        );
    }
    Ok(plan_dir)
}

/// Scaffold the minimum set of files a plan directory must contain
/// before the create-plan LLM session runs. Creates the directory
/// itself (refusing if it already exists) and writes:
///
/// - `phase.md` = `triage\n`
/// - `backlog.yaml` = `schema_version: 1\nitems: []\n`
/// - `intents.yaml` = `schema_version: 1\nitems: []\n`
/// - `memory.yaml` = `schema_version: 1\nitems: []\n`
/// - `dream-word-count` = `0`
///
/// Parent directories are NOT created here — `validate_target` handles
/// that — so this function only succeeds when called against a freshly
/// validated target path.
///
/// After scaffolding, the LLM populates backlog, intents, and memory
/// via `ravel-lite state <area> add` rather than writing YAML directly.
/// This keeps the "no LLM-authored mechanical scaffolding" contract
/// intact.
pub fn scaffold_plan_dir(abs_plan_dir: &Path) -> Result<()> {
    fs::create_dir(abs_plan_dir).with_context(|| {
        format!(
            "Failed to create plan directory {}",
            abs_plan_dir.display()
        )
    })?;

    let writes: [(&str, &[u8]); 6] = [
        (PHASE_FILENAME, b"triage\n"),
        (BACKLOG_FILENAME, b"schema_version: 1\nitems: []\n"),
        (INTENTS_FILENAME, b"schema_version: 1\nitems: []\n"),
        (MEMORY_FILENAME, b"schema_version: 1\nitems: []\n"),
        (DREAM_WORD_COUNT_FILENAME, b"0"),
        (".gitignore", b".worktrees/\n"),
    ];
    for (name, bytes) in writes {
        let path = abs_plan_dir.join(name);
        fs::write(&path, bytes)
            .with_context(|| format!("Failed to write {}", path.display()))?;
    }
    Ok(())
}

/// Write `<plan>/target-requests.yaml` seeded from CLI-supplied
/// `--target` flags. Empty input is a no-op (no file written) so that
/// plans created without `--target` don't carry a stub the runner
/// would parse-and-discard at every phase boundary.
///
/// Per architecture-next §`ravel-lite create <plan>`, target proposal is
/// usually the LLM's job during the create dialogue — but a user who
/// already knows their components can skip that round-trip by passing
/// `--target` flags. The shape (TargetRequestsFile + TargetRequest) is
/// the same one the runner drains at the next phase boundary, so seeded
/// entries flow through `mount_target` identically to LLM-authored ones.
fn seed_target_requests(abs_plan_dir: &Path, targets: &[ComponentRef]) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    let file = TargetRequestsFile {
        schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
        requests: targets
            .iter()
            .map(|component| TargetRequest {
                component: component.clone(),
                reason: "Seeded by `ravel-lite create --target`".to_string(),
            })
            .collect(),
    };
    write_target_requests(abs_plan_dir, &file)
}

pub async fn run_create(
    config_root: &Path,
    plan_name: &str,
    targets: &[ComponentRef],
) -> Result<()> {
    let shared = load_shared_config(config_root)?;
    if shared.agent != "claude-code" {
        anyhow::bail!(
            "create currently only supports agent 'claude-code' (configured agent: '{}').",
            shared.agent
        );
    }

    let abs_plan_dir = resolve_plan_dir(config_root, plan_name)?;
    let plans_dir = config_root.join("plans");
    if !plans_dir.exists() {
        fs::create_dir_all(&plans_dir)
            .with_context(|| format!("Failed to create {}", plans_dir.display()))?;
    }

    // Runner-owned scaffolding runs BEFORE the claude spawn so the LLM
    // never has to create mechanical files (phase.md, empty YAML shells,
    // dream-word-count). The create-plan prompt directs it to populate
    // intents/backlog/memory exclusively through `state intents add` etc.
    // — no raw writes.
    scaffold_plan_dir(&abs_plan_dir)?;
    seed_target_requests(&abs_plan_dir, targets)?;

    let template = require_embedded(CREATE_PLAN_PROMPT_PATH)?;
    let prompt = compose_create_prompt(template, &abs_plan_dir, config_root)?;

    let agent_config = load_agent_config(config_root, &shared.agent)?;
    // Plan creation is work-phase-like reasoning; reuse the configured
    // work model rather than introducing a separate model axis.
    let model = agent_config.models.get("work").cloned().ok_or_else(|| {
        anyhow::anyhow!("Agent config is missing a `models.work` entry; cannot select a model for create.")
    })?;

    eprintln!(
        "Launching interactive claude session (model: {}) to create plan at {}...",
        model,
        abs_plan_dir.display()
    );

    let mut child = TokioCommand::new("claude")
        .arg(&prompt)
        .arg("--model")
        .arg(&model)
        .arg("--add-dir")
        .arg(config_root)
        .arg("--allowed-tools")
        .arg(CREATE_ALLOWED_TOOLS)
        .env("RAVEL_LITE_CONFIG", config_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to spawn claude CLI. Ensure it is installed and on PATH.")?;

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("claude exited with status {status}");
    }

    // Post-hoc verification: scaffolding guarantees the YAML shells
    // exist; a still-empty intents.yaml signals that the LLM session
    // exited before drafting any intents. Backlog stays empty after
    // create — the first triage cycle generates backlog items from
    // intents (per architecture-next §`ravel-lite run <plan>`).
    let intents = crate::state::intents::read_intents(&abs_plan_dir)
        .context("Failed to read scaffolded intents.yaml after claude session")?;
    if intents.items.is_empty() {
        eprintln!(
            "\nwarning: {} has no intents — the session may have exited before the plan was populated.",
            abs_plan_dir.display()
        );
    } else {
        println!("\nPlan created at {}", abs_plan_dir.display());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn compose_prompt_substitutes_plan_token() {
        let template = "Plan path: {{PLAN}}";
        let out = compose_create_prompt(
            template,
            Path::new("/abs/plans/myplan"),
            Path::new("/abs/context"),
        )
        .unwrap();
        assert_eq!(out, "Plan path: /abs/plans/myplan");
    }

    #[test]
    fn compose_prompt_errors_on_unresolved_token() {
        let template = "Bogus: {{NOT_A_TOKEN}}";
        let err = compose_create_prompt(
            template,
            Path::new("/abs/plans/myplan"),
            Path::new("/abs/context"),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("unresolved token"));
    }

    #[test]
    fn compose_prompt_against_real_template_passes_substitution() {
        // The shipped create-plan.md must compose without unresolved tokens.
        let template = crate::init::require_embedded(CREATE_PLAN_PROMPT_PATH).unwrap();
        let out = compose_create_prompt(
            template,
            Path::new("/abs/plans/myplan"),
            Path::new("/abs/context"),
        )
        .unwrap();
        assert!(out.contains("Intent articulation"), "missing §1 marker");
        assert!(out.contains("Target proposal"), "missing §2 marker");
        assert!(out.contains("Anchor capture"), "missing §3 marker");
        assert!(out.contains("/abs/plans/myplan"), "{{PLAN}} did not substitute");
    }

    #[test]
    fn create_prompt_uses_target_requests_verbs_not_raw_yaml_writes() {
        // Regression guard: §2 must instruct the LLM to use the
        // `state target-requests` verbs, not raw `Write` against
        // `target-requests.yaml`. A raw write would (a) clobber any
        // entries seeded by `ravel-lite create --target` and (b) almost
        // always omit `schema_version: 1`, producing a file the runner
        // rejects at the next phase boundary.
        let template = crate::init::require_embedded(CREATE_PLAN_PROMPT_PATH).unwrap();
        assert!(
            template.contains("state target-requests add"),
            "create-plan §2 must instruct using `state target-requests add` to record proposed targets"
        );
        assert!(
            template.contains("state target-requests list"),
            "create-plan §2 must instruct reading the seeded queue via `state target-requests list` before proposing"
        );
    }

    #[test]
    fn create_allowed_tools_includes_target_requests_verbs() {
        // The prompt instructs the LLM to call `state target-requests`
        // verbs; the allowlist must permit them or every invocation
        // would prompt the user during a flow that should be friction-free.
        assert!(
            CREATE_ALLOWED_TOOLS.contains("Bash(ravel-lite state target-requests:*)"),
            "CREATE_ALLOWED_TOOLS must permit `state target-requests` verbs; got: {CREATE_ALLOWED_TOOLS}"
        );
    }

    #[test]
    fn scaffold_plan_dir_creates_directory_and_required_files() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("plan-name");
        scaffold_plan_dir(&plan).unwrap();
        assert!(plan.is_dir(), "plan directory must exist after scaffold");
        assert_eq!(fs::read_to_string(plan.join(PHASE_FILENAME)).unwrap(), "triage\n");
        assert_eq!(
            fs::read_to_string(plan.join(BACKLOG_FILENAME)).unwrap(),
            "schema_version: 1\nitems: []\n"
        );
        assert_eq!(
            fs::read_to_string(plan.join(INTENTS_FILENAME)).unwrap(),
            "schema_version: 1\nitems: []\n"
        );
        assert_eq!(
            fs::read_to_string(plan.join(MEMORY_FILENAME)).unwrap(),
            "schema_version: 1\nitems: []\n"
        );
        assert_eq!(fs::read_to_string(plan.join(DREAM_WORD_COUNT_FILENAME)).unwrap(), "0");
    }

    #[test]
    fn scaffold_plan_dir_writes_gitignore_excluding_worktrees() {
        // architecture-next §Layout calls for `.worktrees/` to be
        // gitignored within the plan directory so per-cycle worktree
        // mounts never get accidentally committed.
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("plan-name");
        scaffold_plan_dir(&plan).unwrap();

        let gitignore = fs::read_to_string(plan.join(".gitignore")).unwrap();
        assert!(
            gitignore.lines().any(|line| line.trim() == ".worktrees/"),
            ".gitignore must list .worktrees/ as a directory pattern; got:\n{gitignore}"
        );
    }

    #[test]
    fn scaffold_plan_dir_writes_cli_parseable_state_files() {
        // The YAML shells must parse via the canonical readers so the
        // LLM's first `state backlog add` / `state intents add` /
        // `state memory add` lands on valid files rather than triggering
        // a format error.
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("plan-name");
        scaffold_plan_dir(&plan).unwrap();

        let backlog = crate::state::backlog::read_backlog(&plan).unwrap();
        assert!(backlog.items.is_empty(), "scaffolded backlog must have no tasks");

        let intents = crate::state::intents::read_intents(&plan).unwrap();
        assert!(intents.items.is_empty(), "scaffolded intents must have no entries");

        let memory = crate::state::memory::read_memory(&plan).unwrap();
        assert!(memory.items.is_empty(), "scaffolded memory must have no entries");
    }

    #[test]
    fn scaffold_plan_dir_refuses_existing_directory() {
        let tmp = TempDir::new().unwrap();
        let err = scaffold_plan_dir(tmp.path()).unwrap_err();
        assert!(
            format!("{err:#}").contains("create plan directory"),
            "scaffold_plan_dir must error when the plan dir already exists; got: {err:#}"
        );
    }

    #[test]
    fn seed_target_requests_writes_supplied_components_into_target_requests_yaml() {
        // CLI-seeded targets land as TargetRequest entries that the
        // first phase boundary will drain into mounted worktrees.
        use crate::component_ref::ComponentRef;
        use crate::state::target_requests::read_target_requests;

        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("plan-name");
        scaffold_plan_dir(&plan).unwrap();

        let targets = vec![
            ComponentRef::new("atlas", "atlas-core"),
            ComponentRef::new("ravel-lite", "phase-loop"),
        ];
        seed_target_requests(&plan, &targets).unwrap();

        let parsed = read_target_requests(&plan).unwrap();
        assert_eq!(parsed.requests.len(), 2);
        assert_eq!(
            parsed.requests[0].component,
            ComponentRef::new("atlas", "atlas-core")
        );
        assert_eq!(
            parsed.requests[1].component,
            ComponentRef::new("ravel-lite", "phase-loop")
        );
        assert!(
            !parsed.requests[0].reason.is_empty(),
            "TargetRequest.reason must be non-empty (verbs::run_add enforces this for LLM-authored entries)"
        );
    }

    #[test]
    fn seed_target_requests_with_empty_list_is_a_no_op() {
        // The CLI flag is optional; create without --target must not
        // leave behind an empty target-requests.yaml that the runner
        // would then parse-and-discard each phase boundary.
        use crate::state::target_requests::yaml_io::target_requests_path;

        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("plan-name");
        scaffold_plan_dir(&plan).unwrap();

        seed_target_requests(&plan, &[]).unwrap();

        assert!(
            !target_requests_path(&plan).exists(),
            "no target-requests.yaml should be written when --target is absent"
        );
    }

    #[test]
    fn seed_target_requests_reason_identifies_create_time_origin() {
        // A human reading the queue (or triage) needs to be able to
        // distinguish CLI-seeded entries from later LLM-authored ones.
        use crate::component_ref::ComponentRef;
        use crate::state::target_requests::read_target_requests;

        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("plan-name");
        scaffold_plan_dir(&plan).unwrap();

        let targets = vec![ComponentRef::new("atlas", "atlas-core")];
        seed_target_requests(&plan, &targets).unwrap();

        let parsed = read_target_requests(&plan).unwrap();
        assert!(
            parsed.requests[0].reason.to_lowercase().contains("create"),
            "reason must identify create-time origin; got: {}",
            parsed.requests[0].reason
        );
    }

    #[test]
    fn validate_plan_name_accepts_simple_alphanumeric() {
        assert!(validate_plan_name("foo").is_ok());
        assert!(validate_plan_name("foo-bar").is_ok());
        assert!(validate_plan_name("foo_bar.v2").is_ok());
        assert!(validate_plan_name("plan123").is_ok());
    }

    #[test]
    fn validate_plan_name_rejects_empty() {
        assert!(validate_plan_name("").is_err());
    }

    #[test]
    fn validate_plan_name_rejects_path_separators() {
        assert!(validate_plan_name("foo/bar").is_err());
        assert!(validate_plan_name("foo\\bar").is_err());
    }

    #[test]
    fn validate_plan_name_rejects_dot_prefix() {
        assert!(validate_plan_name(".foo").is_err());
        assert!(validate_plan_name(".").is_err());
    }

    #[test]
    fn validate_plan_name_rejects_dash_prefix() {
        assert!(validate_plan_name("-foo").is_err());
    }

    #[test]
    fn validate_plan_name_rejects_dot_dot() {
        assert!(validate_plan_name("..").is_err());
        assert!(validate_plan_name("foo..bar").is_err());
    }

    #[test]
    fn validate_plan_name_rejects_whitespace() {
        assert!(validate_plan_name("foo bar").is_err());
        assert!(validate_plan_name("foo\tbar").is_err());
    }

    #[test]
    fn validate_plan_name_rejects_invalid_git_ref_chars() {
        for name in ["foo:bar", "foo*bar", "foo?bar", "foo[bar", "foo~bar", "foo^bar"] {
            assert!(validate_plan_name(name).is_err(), "expected reject: {name}");
        }
    }

    #[test]
    fn validate_plan_name_rejects_lock_suffix() {
        assert!(validate_plan_name("foo.lock").is_err());
    }

    #[test]
    fn resolve_plan_dir_joins_under_plans_subdir() {
        let tmp = TempDir::new().unwrap();
        let resolved = resolve_plan_dir(tmp.path(), "foo").unwrap();
        assert_eq!(resolved, tmp.path().join("plans").join("foo"));
    }

    #[test]
    fn resolve_plan_dir_rejects_existing_path() {
        let tmp = TempDir::new().unwrap();
        let plans = tmp.path().join("plans");
        fs::create_dir_all(plans.join("foo")).unwrap();
        let err = resolve_plan_dir(tmp.path(), "foo").unwrap_err();
        assert!(format!("{err:#}").contains("already exists"));
    }

    #[test]
    fn resolve_plan_dir_rejects_invalid_name() {
        let tmp = TempDir::new().unwrap();
        let err = resolve_plan_dir(tmp.path(), ".foo").unwrap_err();
        assert!(format!("{err:#}").contains("cannot start with"));
    }

    #[test]
    fn resolve_plan_dir_does_not_create_directories() {
        let tmp = TempDir::new().unwrap();
        let resolved = resolve_plan_dir(tmp.path(), "foo").unwrap();
        assert!(!resolved.exists(), "resolve_plan_dir must not create the plan dir");
        assert!(!resolved.parent().unwrap().exists(), "must not create plans/ either");
    }
}
