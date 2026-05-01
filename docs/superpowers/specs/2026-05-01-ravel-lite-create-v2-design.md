# Design: v2 `ravel-lite create <plan>` (slice B)

**Date:** 2026-05-01
**Backlog id:** `ravel-lite-create-write-plans-into-context-plans-dir`
**Reference:** `docs/architecture-next.md` §`ravel-lite create <plan>`, §Layout, §Plan lifecycle

## Goal

Re-shape `ravel-lite create` so plans live at `<context>/plans/<plan>/` and the create session has v2's three structured deliverables: intent articulation, target proposal, anchor capture. The v1 shape (`create <full-plan-path>`) is removed.

This is **slice B** of the broader v2 reshape — the slice that delivers the v2 user-facing experience without role-flipping `phase.md` (which is gated on the migrator) and without wiring runner-side consumption of `target-requests.yaml` (which is the `targets-and-worktrees` task's job).

## Out of scope (explicit deferrals)

- `target-requests.yaml` runner-side consumption — `targets-and-worktrees` task.
- Backlog generation from intents at first-cycle triage — `phase-cycle-triage-first-with-focus-objections-and-commits-yaml` task.
- `phase.md` role-flip from phase tracker to rendered overview — `state-intents-render-overview` task.
- Migrating existing v1 plans (`LLM_STATE/*`) to v2 layout — `migrate-ravel-lite-migrate-for-v1-to-v2-cutover` task.
- External-link justifications via CLI flag — current `state intents add` only takes a claim plus single rationale body. Issue-tracker URLs go inline in the rationale body for now.

The v2 plans this code creates are operable only by a future v2 binary. Running `ravel-lite run` against them with the running v1 binary will fail (empty backlog, etc.). This is intentional per the architecture-next "hard cutover" decision.

## Components

### 1. CLI shape (`src/main.rs`)

```rust
Create {
    #[arg(long)]
    config: Option<PathBuf>,
    plan: String,                    // was plan_dir: PathBuf
}
```

`<plan>` is a name, not a path. Path resolution is done in `create.rs` against the resolved context root (the existing `--config` discovery chain — flag, env, default).

### 2. Plan-name validation (`src/create.rs`)

New `validate_plan_name(name: &str) -> Result<()>`. Accepts:

- Non-empty
- Contains only characters valid in a single git ref component
- Doesn't start with `.` or `-`
- Contains no whitespace
- Contains no `/` or `\`
- Is not `.` or `..`
- Contains no `..` substring
- Doesn't contain `~`, `^`, `:`, `?`, `*`, `[`, control chars
- Doesn't end with `.lock`

Rationale: plan names appear in `ravel-lite/<plan>/main` branch refs (per architecture-next §Mounting), commit messages, survey output, `targets.yaml`. Restricting to git-ref-component shape avoids cascading footguns.

### 3. Path resolution (`src/create.rs`)

New `resolve_plan_dir(context_root: &Path, plan_name: &str) -> Result<PathBuf>`:

- Calls `validate_plan_name`.
- Returns `<context_root>/plans/<plan_name>/`.
- Errors if the resolved path already exists.

The existing `validate_target` is removed (only caller was `run_create`).

### 4. Scaffold (unchanged)

`scaffold_plan_dir` continues to write the same five files:

- `phase.md` = `work\n`
- `backlog.yaml` = `schema_version: 1\nitems: []\n`
- `intents.yaml` = `schema_version: 1\nitems: []\n`
- `memory.yaml` = `schema_version: 1\nitems: []\n`
- `dream-word-count` = `0`

`target-requests.yaml` and `anchors.yaml` are **not** scaffolded — the LLM writes them as deliverables. Empty stub files would be misleading.

### 5. Spawn argv (`src/create.rs::run_create`)

```text
claude <prompt>
  --model <work-model>
  --add-dir <context_root>
  --allowed-tools "Bash(ravel-lite atlas:*),
                   Bash(ravel-lite repo:*),
                   Bash(ravel-lite state intents:*),
                   Bash(ravel-lite state backlog:*),
                   Bash(ravel-lite state memory:*),
                   Read,Write,Glob,Grep"
```

Plus `RAVEL_LITE_CONFIG=<context_root>` set on the child env so spawned `ravel-lite` calls inherit the context.

The single `--add-dir <context_root>` replaces the v1 `--add-dir <parent>`. `<context_root>` contains `plans/<plan>/`, `repos.yaml`, registered repos' `local_path` (read by atlas), and `agents/`/`phases/` (overlay paths). Note: registered repos may have `local_path` outside the context root — reading those is needed for `ravel-lite atlas describe` / `summary` / `list-components`. Handled by atlas internally; `--add-dir` only governs claude's direct file access. If the LLM needs to `Read` files inside a registered repo directly (rare), the prompt directs it through atlas instead.

(Per the auto-memory entry "Claude spawn with file-write needs `--allowed-tools`": `--setting-sources project,local` silently blocks Writes in `-p` mode, but `create` is interactive — not `-p` — so the allowed-tools list is for explicit gating rather than unblocking writes. Including it makes the surface explicit and matches the discover-stage spawns' pattern.)

### 6. Prompt re-author (`defaults/create-plan.md`)

Full rewrite. Structure:

- **§0 Invariant: this session produces a v2 plan.** Whatever the user describes is plan scope, not a task to execute. v2 plans are intent-shaped — a bug-fix is one intent ("fix X because Y"), not a backlog task pre-filled at create time. Triage will generate the backlog at first cycle.
- **§1 Intent articulation.** Dialogue to draft 1–5 strategic intents. Each recorded via `ravel-lite state intents add {{PLAN}} --claim "..." --body-file <path>`. Body is a markdown rationale citing the user's stated reason; issue-tracker URLs go inline.
- **§2 Target proposal.** Query atlas (`ravel-lite atlas list-repos`, `list-components`, `summary`, `describe`) to discover candidate components serving each intent. Write `{{PLAN}}/target-requests.yaml` with the schema documented in `docs/architecture-next.md` §Dynamic mounting:
  ```yaml
  requests:
    - component: <repo_slug>:<component_id>
      reason: <text>
  ```
  Show user; accept corrections.
- **§3 Anchor capture.** Components likely-read-but-not-edited recorded in `{{PLAN}}/anchors.yaml`:
  ```yaml
  anchors:
    - component: <repo_slug>:<component_id>
      reason: <text>
  ```
  Same review-with-user gate.
- **§4 Review and exit.** Show all three artifacts; user approves; session exits. No git commit step.

Token substitution: the prompt template uses `{{PLAN}}` (absolute plan path) and `{{ORCHESTRATOR}}` (path to the running ravel-lite repo). `compose_create_prompt` invokes `substitute_tokens` before appending the instruction block — same canonical path used by phase prompts (per memory entry "All prompt loading routes through `substitute_tokens`"). Today's `compose_create_prompt` uses ad-hoc `format!` and bypasses `substitute_tokens`; this slice fixes that.

Removes from current prompt: single-task-plan language, references to LLM_STATE layout, references to `prompt-work.md`/`pre-work.sh` (orthogonal to this slice; v2 plan-local prompt overrides are a separate concern handled by the v2 phase cycle), §4 "Write the files" (scaffolding done before spawn), §5 "Commit" (orchestrator-side).

### 7. Verification post-spawn

`run_create` replaces "backlog non-empty" check with "intents non-empty":

```rust
let intents = state::intents::read_intents(&abs_plan_dir)?;
if intents.items.is_empty() {
    eprintln!(
        "warning: {} has no intents — the session may have exited early.",
        abs_plan_dir.display()
    );
}
```

No hard error — same advisory pattern as today.

`target-requests.yaml` and `anchors.yaml` existence is **not** verified post-spawn — both are legitimately optional (a plan with no ambitions to mount anything yet, or no read-only components, can have neither).

## Data flow

```
user types: ravel-lite create my-plan --config ~/.config/ravel-lite/

  ↓ main.rs: Commands::Create { config, plan }
  ↓ resolve_config_root → context_root
  ↓ create::run_create(&context_root, plan)

run_create:
  ↓ resolve_plan_dir(context_root, plan)
  ↓   validate_plan_name(plan)
  ↓   join, check non-existent
  ↓ scaffold_plan_dir(abs_plan_dir)
  ↓ compose_create_prompt(template, abs_plan_dir, context_root)
  ↓ spawn claude (interactive, inherits stdio)
  ↓   LLM dialogues with user
  ↓   LLM writes intents.yaml (via state intents add)
  ↓   LLM writes target-requests.yaml (via Write)
  ↓   LLM writes anchors.yaml (via Write)
  ↓ wait for child exit
  ↓ verify: intents non-empty (warning if not)

Final tree at <context_root>/plans/<plan>/:
  phase.md
  backlog.yaml          (empty)
  intents.yaml          (1-5 entries)
  memory.yaml           (empty)
  dream-word-count
  target-requests.yaml  (LLM-authored, optional)
  anchors.yaml          (LLM-authored, optional)
```

## Test strategy

- **Unit (validate_plan_name):** ~10 cases — accept (`foo`, `foo-bar`, `foo_bar.v2`); reject (empty, `.foo`, `-foo`, `foo/bar`, `foo bar`, `..`, `foo..bar`, `foo*`, `foo.lock`, `foo:bar`).
- **Unit (resolve_plan_dir):** existing-path rejection; correct join under `<ctx>/plans/`; passes name validation through.
- **Unit (scaffold_plan_dir):** unchanged from today.
- **Unit (compose_create_prompt):** assertions for the §1/§2/§3 deliverable markers and `{{PLAN}}` token substitution.
- **Integration:** `ravel-lite create <plan>` from CLI, with claude binary stubbed out (or absent — failure path tested separately) — verifies path resolution and scaffold land correctly.
- **No live claude integration test** for session contents — same constraint as `survey` and `discover`; covered by prompt-author review.

## Risks and unknowns

- **R1: Empty backlog + v1 phase loop = inert.** Acknowledged. v2 plans are inert until v2 binary ships. Mitigation: documented in this design; not a concern for slice scope.
- **R2: `state intents add` doesn't yet take external-link justifications.** Issue-tracker URLs land inline in rationale body. Mitigation: design accommodates; CLI extension is a future task if friction surfaces.
- **R3: Allowed-tools breadth.** If the prompt evolves to need `Bash(ravel-lite plan:*)` (plan inspect) or `Bash(ravel-lite state findings:*)`, the allowlist must be extended in lockstep with prompt edits. Mitigation: add only when the prompt actually uses them; today's surface is the minimum for the three deliverables.
- **R4: Atlas `local_path` resolution outside `--add-dir`.** Atlas reads `<repo>/.atlas/` from the registered `local_path`, which is typically outside the context root. Atlas does this internally (via subprocess output back to the LLM); claude does not need direct file access to those paths. Mitigation: prompt directs catalog queries through `ravel-lite atlas` rather than direct file reads.

## Files touched

| Path | Change |
|---|---|
| `src/main.rs` | `Create` enum: `plan_dir: PathBuf` → `plan: String`. Update dispatch. |
| `src/create.rs` | Add `validate_plan_name`, `resolve_plan_dir`. Remove `validate_target`. Update `compose_create_prompt` to invoke `substitute_tokens` (no signature change). Update `run_create` signature (`plan: &str`, not `plan_dir: PathBuf`) and body: spawn argv, env, post-spawn verification. Extend the existing inline `#[cfg(test)] mod tests` with the unit tests in §Test strategy. |
| `defaults/create-plan.md` | Full rewrite per §6 above. |
| `src/init.rs` | No source change expected. The drift-detection test (`every_file_under_defaults_is_registered_in_embedded_files`) will catch any byte-mismatch; embedded path is unchanged. |

No new modules, no new crate dependencies, no schema changes to existing typed YAML stores.
