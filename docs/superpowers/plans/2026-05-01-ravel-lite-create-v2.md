# v2 `ravel-lite create` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-shape `ravel-lite create` so plans live at `<context>/plans/<plan>/`, the LLM session has three structured deliverables (intents, target-requests, anchors), and the spawned claude has the atlas + state CLI surfaces pre-approved.

**Architecture:** Two new pure helpers (`validate_plan_name`, `resolve_plan_dir`) replace `validate_target`. The CLI takes a plan *name*, not a path. `run_create` resolves under the existing `--config` discovery chain (now interpreted as the v2 context root). The prompt is rewritten to drive the three deliverables and the prompt-composition switches from ad-hoc `format!` to canonical `substitute_tokens`.

**Tech Stack:** Rust 2021, anyhow, clap, tokio, serde_yaml. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-01-ravel-lite-create-v2-design.md`

---

## Note on commits in this work-phase

This plan is being executed inside a ravel-lite work-phase. The work-phase contract says **do not commit source-file changes** — the analyse-work phase that runs immediately after captures the dirty tree and commits everything outside `LLM_STATE/core/`. Skip every `git commit` step in this plan; the TDD test/verify discipline is preserved.

If this plan is later executed outside a work-phase (e.g. ad-hoc), restore commits at task boundaries.

---

## File structure

| File | Responsibility |
|---|---|
| `src/create.rs` | New: `validate_plan_name`, `resolve_plan_dir`. Updated: `compose_create_prompt`, `run_create`. Removed: `validate_target`. New tests inline. |
| `src/main.rs` | `Create` enum field rename (`plan_dir: PathBuf` → `plan: String`); dispatch site adjustment. |
| `defaults/create-plan.md` | Full rewrite to v2 three-deliverable structure. |

`src/init.rs` is touched only by the embed registry (no source change expected — same path, new content). The `every_file_under_defaults_is_registered_in_embedded_files` drift test catches any byte-mismatch.

---

## Task 1: Add `validate_plan_name` (TDD)

**Files:**
- Modify: `src/create.rs` (add tests + new function)

- [ ] **Step 1.1: Write failing tests**

Append to the `#[cfg(test)] mod tests` block in `src/create.rs`:

```rust
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
```

- [ ] **Step 1.2: Run tests to verify they fail**

Run: `cargo test --package ravel-lite --lib create::tests::validate_plan_name -- --nocapture`
Expected: compile error (function `validate_plan_name` not in scope).

- [ ] **Step 1.3: Add function**

Insert into `src/create.rs` near the top of the public function block (above `validate_target`):

```rust
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
```

- [ ] **Step 1.4: Run tests to verify they pass**

Run: `cargo test --package ravel-lite --lib create::tests::validate_plan_name`
Expected: 9 tests pass.

---

## Task 2: Add `resolve_plan_dir` (TDD)

**Files:**
- Modify: `src/create.rs` (add tests + new function)

- [ ] **Step 2.1: Write failing tests**

Append to `#[cfg(test)] mod tests`:

```rust
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
```

- [ ] **Step 2.2: Run tests to verify they fail**

Run: `cargo test --package ravel-lite --lib create::tests::resolve_plan_dir`
Expected: compile error (`resolve_plan_dir` not in scope).

- [ ] **Step 2.3: Add function**

Insert into `src/create.rs` above `validate_target`:

```rust
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
```

- [ ] **Step 2.4: Run tests to verify they pass**

Run: `cargo test --package ravel-lite --lib create::tests::resolve_plan_dir`
Expected: 4 tests pass.

---

## Task 3: Switch CLI shape and `run_create` signature

**Files:**
- Modify: `src/main.rs` (Create enum + dispatch)
- Modify: `src/create.rs` (run_create signature + body, remove validate_target)

This task is non-TDD because the change is structural (signature + dispatch) and tested by build success. The new helpers from Tasks 1-2 are the unit-tested layer.

- [ ] **Step 3.1: Update `Create` enum in `src/main.rs`**

Find the `Create` variant (around line 137):

```rust
Create {
    #[arg(long)]
    config: Option<PathBuf>,
    plan_dir: PathBuf,
}
```

Replace `plan_dir: PathBuf` with `plan: String`:

```rust
Create {
    #[arg(long)]
    config: Option<PathBuf>,
    /// Plan name. Resolved to <context_root>/plans/<plan>/.
    /// See validate_plan_name for accepted characters.
    plan: String,
}
```

- [ ] **Step 3.2: Update dispatch site in `src/main.rs`**

Find (around line 1211):

```rust
Commands::Create { config, plan_dir } => {
    let config_root = resolve_config_root(config)?;
    create::run_create(&config_root, plan_dir).await
}
```

Replace with:

```rust
Commands::Create { config, plan } => {
    let config_root = resolve_config_root(config)?;
    create::run_create(&config_root, &plan).await
}
```

- [ ] **Step 3.3: Update `run_create` signature and body in `src/create.rs`**

Change signature from `pub async fn run_create(config_root: &Path, plan_dir: PathBuf) -> Result<()>` to `pub async fn run_create(config_root: &Path, plan_name: &str) -> Result<()>`.

In the body, replace:

```rust
let abs_plan_dir = validate_target(&plan_dir)?;
let parent = abs_plan_dir
    .parent()
    .expect("validated parent exists")
    .to_path_buf();
```

With:

```rust
let abs_plan_dir = resolve_plan_dir(config_root, plan_name)?;
```

(Drop the `parent` local — Task 4 will switch `--add-dir` to `config_root`, no parent extraction needed.)

Below this, the `--add-dir` arg site currently uses `&parent`. Temporarily change to `config_root` in this task too (formal switch is Task 4 alongside the allowed-tools addition):

Find:

```rust
.arg("--add-dir")
.arg(&parent)
```

Replace:

```rust
.arg("--add-dir")
.arg(config_root)
```

- [ ] **Step 3.4: Remove `validate_target` and its tests**

Delete the `validate_target` function from `src/create.rs`.

Delete these tests from the `mod tests` block:
- `validate_target_rejects_existing_directory`
- `validate_target_rejects_existing_file`
- `validate_target_creates_missing_parent_directories`
- `validate_target_rejects_when_parent_is_a_file`
- `validate_target_accepts_new_path_under_existing_parent`

- [ ] **Step 3.5: Verify build and tests**

Run: `cargo build`
Expected: clean compile.

Run: `cargo test --package ravel-lite --lib create::tests`
Expected: all remaining create tests pass (the validate_plan_name + resolve_plan_dir + scaffold_plan_dir + compose_prompt tests; the validate_target tests are gone). Note: `compose_prompt_*` tests will still pass because the prompt is unchanged at this point.

---

## Task 4: Update spawn argv (`--allowed-tools`, env var)

**Files:**
- Modify: `src/create.rs` (run_create body)

- [ ] **Step 4.1: Add allowed-tools constant**

Near the top of `src/create.rs` below the `CREATE_PLAN_PROMPT_PATH` constant, add:

```rust
/// Tools pre-approved for the create session. Restricting Bash to
/// specific ravel-lite invocations prevents accidental shell-out during
/// the dialogue while keeping the catalog and state CLIs friction-free.
const CREATE_ALLOWED_TOOLS: &str = "Bash(ravel-lite atlas:*),\
Bash(ravel-lite repo:*),\
Bash(ravel-lite state intents:*),\
Bash(ravel-lite state backlog:*),\
Bash(ravel-lite state memory:*),\
Read,Write,Glob,Grep";
```

- [ ] **Step 4.2: Update spawn argv**

In `run_create`, find the spawn block:

```rust
let mut child = TokioCommand::new("claude")
    .arg(&prompt)
    .arg("--model")
    .arg(&model)
    .arg("--add-dir")
    .arg(config_root)
    .stdin(Stdio::inherit())
    .stdout(Stdio::inherit())
    .stderr(Stdio::inherit())
    .spawn()
    .context("Failed to spawn claude CLI. Ensure it is installed and on PATH.")?;
```

Replace with:

```rust
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
```

- [ ] **Step 4.3: Verify build**

Run: `cargo build`
Expected: clean compile. (Spawn argv is not unit-tested directly — covered by the `survey`/`discover` patterns and downstream integration.)

---

## Task 5: Rewrite prompt + switch `compose_create_prompt` to `substitute_tokens`

**Files:**
- Modify: `defaults/create-plan.md` (full rewrite)
- Modify: `src/create.rs` (`compose_create_prompt` body + signature; tests)

These two changes are atomic — the new prompt body uses `{{PLAN}}` tokens that the old `compose_create_prompt` does not substitute, so the prompt rewrite and the substitute-tokens switch must land together.

- [ ] **Step 5.1: Write failing tests for new prompt structure**

In `src/create.rs` `mod tests`, **replace** the existing three `compose_prompt_*` tests with:

```rust
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
    // Required structural markers from the v2 prompt:
    assert!(out.contains("Intent articulation"), "missing §1 marker");
    assert!(out.contains("Target proposal"), "missing §2 marker");
    assert!(out.contains("Anchor capture"), "missing §3 marker");
    assert!(out.contains("/abs/plans/myplan"), "{{PLAN}} did not substitute");
}
```

- [ ] **Step 5.2: Run tests to verify they fail**

Run: `cargo test --package ravel-lite --lib create::tests::compose_prompt`
Expected: compile errors (signature mismatch — old `compose_create_prompt` takes 2 args). Plus: removed tests no longer reference the old assertions.

- [ ] **Step 5.3: Rewrite `compose_create_prompt`**

In `src/create.rs`, replace the existing `compose_create_prompt` function with:

```rust
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
    use crate::types::PlanContext;
    use std::collections::HashMap;

    let ctx = PlanContext {
        dev_root: String::new(),
        project_dir: context_root.display().to_string(),
        plan_dir: abs_plan_dir.display().to_string(),
        config_root: context_root.display().to_string(),
        related_plans: String::new(),
    };
    let tokens: HashMap<String, String> = HashMap::new();
    crate::prompt::substitute_tokens(template, &ctx, &tokens)
}
```

Note: this changes the return type from `String` to `Result<String>`.

- [ ] **Step 5.4: Update `run_create` to handle the new return**

In `src/create.rs`, find:

```rust
let template = require_embedded(CREATE_PLAN_PROMPT_PATH)?;
let prompt = compose_create_prompt(template, &abs_plan_dir);
```

Replace with:

```rust
let template = require_embedded(CREATE_PLAN_PROMPT_PATH)?;
let prompt = compose_create_prompt(template, &abs_plan_dir, config_root)?;
```

- [ ] **Step 5.5: Rewrite `defaults/create-plan.md`**

Replace the entire content of `defaults/create-plan.md` with:

```markdown
# Creating a v2 plan

You're a headful claude session driving a dialogue with the user to
populate a new ravel-lite v2 plan at `{{PLAN}}`. The plan directory has
already been scaffolded by `ravel-lite create`. This session has three
structured deliverables.

## §0. Invariant: this session produces a plan

Your ONLY output from this session is a populated plan directory at
`{{PLAN}}`. Whatever the user describes is the plan's scope, not a task
for you to execute now.

A v2 plan is intent-shaped, not task-shaped. A bug-fix plan is one
strategic intent ("fix X because Y") with a justification linking to
the user's stated reason. The backlog is filled at the first triage
cycle from those intents — not pre-filled here.

Do NOT attempt to do the work the user describes (e.g. fix the bug,
implement the feature). Your job is to draft three artifacts (intents,
target requests, anchors) and confirm them with the user. When in
doubt, the right response is "I'll capture that as an intent at
`{{PLAN}}`; what other intents belong alongside it?"

## §1. Intent articulation

Dialogue with the user to draft 1–5 strategic intents that the plan
exists to pursue.

For each intent:
- The **claim** is a one-sentence statement of what success looks like.
- The **justification** is a markdown rationale citing the user's stated
  reason. Include any issue-tracker URLs inline in the rationale.

Record each intent via:

    ravel-lite state intents add {{PLAN}} \
      --claim "<one-sentence claim>" \
      --body-file <path-to-rationale.md>

Write the rationale to a temp file first (use the Write tool); do not
attempt multi-line `--body` inline.

Reject intent-shaped tasks (e.g. "fix bug X" alone) — those are backlog
items. Push the user toward a strategic framing ("X is broken because
Y; we want it fixed for Z").

Show the result with `ravel-lite state intents list {{PLAN}}` and
confirm with the user before continuing.

## §2. Target proposal

For each intent, identify which components in the registered repos
likely need editing to satisfy it. Use the atlas CLI for catalog
queries:

- `ravel-lite atlas list-repos` — enumerate registered repos.
- `ravel-lite atlas list-components --repo <slug>` — list components in
  a repo.
- `ravel-lite atlas summary --repo <slug>` — high-level repo overview.
- `ravel-lite atlas describe <repo>:<component>` — component details.
- `ravel-lite atlas neighbors <repo>:<component>` — connected components.

Write the proposed targets to `{{PLAN}}/target-requests.yaml`:

    requests:
      - component: <repo_slug>:<component_id>
        reason: <one-sentence reason this component serves the intent>

Show the file to the user; accept corrections by editing the file.

If no concrete targets are knowable yet (rare — usually means the plan
needs more clarification on §1), the file may be omitted. Note this to
the user explicitly.

## §3. Anchor capture

Components mentioned in the conversation that the plan likely *reads
but does not edit* are recorded as anchors — graph-RAG starting points
for later triage cycles.

Write to `{{PLAN}}/anchors.yaml`:

    anchors:
      - component: <repo_slug>:<component_id>
        reason: <one-sentence reason this component is referenced but not edited>

Show the file to the user; accept corrections by editing the file. If
no read-only references surfaced in the conversation, the file may be
omitted.

## §4. Review and exit

Show all three artifacts (intents via `ravel-lite state intents list
{{PLAN}}`, target-requests by reading the file, anchors by reading the
file) and confirm with the user. Once approved, exit. The user will
commit the plan directory separately.
```

- [ ] **Step 5.6: Run tests to verify they pass**

Run: `cargo test --package ravel-lite --lib create::tests::compose_prompt`
Expected: 3 tests pass.

Run: `cargo test --package ravel-lite --test embedded_defaults_drift`
Expected: pass (the embed registry path didn't change; only content).

---

## Task 6: Switch verification: backlog → intents

**Files:**
- Modify: `src/create.rs` (run_create post-spawn block)

- [ ] **Step 6.1: Update post-spawn verification**

In `src/create.rs::run_create`, find:

```rust
// Post-hoc verification: scaffolding guarantees phase.md exists
// from the pre-spawn write, so a still-empty backlog signals that
// the LLM session exited before populating any tasks. Anything
// stricter (e.g. requiring N tasks) would fight single-task plans.
let backlog = crate::state::backlog::read_backlog(&abs_plan_dir)
    .context("Failed to read scaffolded backlog.yaml after claude session")?;
if backlog.items.is_empty() {
    eprintln!(
        "\nwarning: {} still has no tasks — the session may have exited before the plan was populated.",
        abs_plan_dir.display()
    );
} else {
    println!("\nPlan created at {}", abs_plan_dir.display());
}
```

Replace with:

```rust
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
```

- [ ] **Step 6.2: Verify build**

Run: `cargo build`
Expected: clean compile.

---

## Task 7: Final verification

**Files:** none (verification-only)

- [ ] **Step 7.1: Run full test suite**

Run: `cargo test --workspace`
Expected: all tests pass. Specific tests touched by this plan: `create::tests::*`, `embedded_defaults_drift`. Tests not touched should continue to pass.

- [ ] **Step 7.2: Run the `scripts/check.sh` gate**

Run: `./scripts/check.sh`
Expected: pass. Per README.md, this is the single source of truth for "what must pass before main" (currently `cargo clippy --all-targets --workspace -- -D warnings`).

- [ ] **Step 7.3: Smoke-check the new prompt content**

Read `defaults/create-plan.md` and verify visually:
- Three deliverable section headers: "§1. Intent articulation", "§2. Target proposal", "§3. Anchor capture".
- The only `{{NAME}}`-shaped tokens present are `{{PLAN}}` (substituted by `compose_create_prompt`).
- No leftover v1 tokens (`{{PROJECT}}`, `{{DEV_ROOT}}`, `{{ORCHESTRATOR}}`, `{{RELATED_PLANS}}`, `{{TOOL_READ}}`).

Cross-check with: `grep -oE '\{\{[A-Z_]+\}\}' defaults/create-plan.md | sort -u`
Expected output: `{{PLAN}}` (and only that).

- [ ] **Step 7.4: Confirm dirty tree contents**

Run: `git status -s`
Expected dirty paths:
- `src/create.rs`
- `src/main.rs`
- `defaults/create-plan.md`
- `docs/superpowers/specs/2026-05-01-ravel-lite-create-v2-design.md`
- `docs/superpowers/plans/2026-05-01-ravel-lite-create-v2.md`

The analyse-work phase will commit these.

---

## Spec coverage check

| Spec section | Implementing task |
|---|---|
| §1 CLI shape (Create enum) | Task 3 |
| §2 Plan-name validation | Task 1 |
| §3 Path resolution | Task 2 |
| §4 Scaffold (unchanged) | (no task — already correct) |
| §5 Spawn argv | Task 4 (allowed-tools, env) + Task 3 (--add-dir switch) |
| §6 Prompt rewrite + token substitution | Task 5 |
| §7 Verification swap | Task 6 |
| §Test strategy | Tasks 1, 2, 5 (inline tests); Task 7 (workspace + check.sh) |
| §R1 (empty backlog + v1) | Acknowledged in spec; no code action |
| §R2 (no external-link justification CLI) | Acknowledged; prompt §1 instructs URLs inline |
| §R3 (allowed-tools breadth) | Task 4 sets minimal surface |
| §R4 (atlas local_path outside --add-dir) | Prompt §2 directs queries through atlas CLI |
