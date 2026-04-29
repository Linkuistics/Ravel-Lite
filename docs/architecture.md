# Architecture

`ravel-lite` is a single Rust binary with a Ratatui TUI. It orchestrates
a phase loop for LLM-driven development by spawning a Claude Code or Pi
subprocess per phase, reading its JSON stream output, and rendering
progress.

## Principles

- **No magic.** All config, prompts, phase state, and memory live as
  readable files on disk. The binary reads them at runtime. Default
  content is embedded in the binary only so that `init` can write it to
  disk — nothing is read from embedded content during `run`.
- **Visible, auditable, adjustable.** Every input the orchestrator uses
  is a file the user can inspect and edit. Every state transition
  writes to the filesystem.
- **Agents are subprocesses.** The orchestrator spawns `claude` or `pi`
  CLI processes, reads their JSON stream output, and renders progress.
  It never calls LLM APIs directly.
- **Configuration drives everything.** Agent selection, per-phase
  model choice, and prompt customisation all live in the config
  directory — embedded YAML defaults plus optional `config.lua`
  override layers (see *Configuration*). The handful of CLI knobs
  that exist (`--dangerous`, `--model` for `survey`) are escape
  hatches that mutate the loaded config in-process; they don't
  introduce parallel sources of truth.

## Overview

```
┌─────────────────────────────────────────────────────┐
│  main()                                              │
│  Parse CLI args, load config, build PlanContext      │
│  Create agent, create TUI, run phase_loop            │
└──────────────────────┬───────────────────────────────┘
                       │
          ┌────────────▼────────────┐
          │      phase_loop         │
          │  Reads phase.md         │
          │  Drives state machine   │
          │  Calls agent methods    │
          │  Sends events to TUI    │
          └────┬───────────┬────────┘
               │           │
    ┌──────────▼──┐   ┌────▼──────────┐
    │   Agent     │   │     TUI       │
    │  (trait)    │   │  (Ratatui)    │
    │             │   │               │
    │ Spawns CLI  │   │ Renders log,  │
    │ Parses JSON │   │ progress,     │
    │ Emits events│   │ status bar    │
    └─────────────┘   └───────────────┘
```

## Message Model

All communication to the TUI flows through a single
`mpsc::UnboundedSender<UIMessage>` channel. Both agents and the phase
loop send messages on the same channel.

```rust
pub enum UIMessage {
    // ── From agents ──────────────────────────────────────

    /// Overwritable progress for an agent (latest tool call).
    /// Carried as a `StyledLine` so the format module can attach
    /// semantic colour intents (Added/Removed/Changed/Meta) without
    /// reaching for ratatui types directly.
    Progress { agent_id: String, line: StyledLine },

    /// Permanent styled output for an agent — highlight labels,
    /// result text with action markers, tool errors. Inserted into
    /// scrollback line-by-line via `Terminal::insert_before`. Side
    /// effect: clears this agent's progress line in the live area.
    Persist { agent_id: String, lines: Vec<StyledLine> },

    /// Agent finished. Removes its progress group from the live area.
    AgentDone { agent_id: String },

    // ── From the phase loop ──────────────────────────────

    /// Permanent plain-text output (phase headers, commit summaries,
    /// orchestrator warnings). The string may contain `\n`; it's
    /// split and inserted into scrollback line-by-line. Side effect:
    /// clears every agent's progress line, since a phase-level log
    /// invalidates whatever tool was reportedly in flight.
    Log(String),

    /// Register an agent group so subsequent `Progress` events for
    /// the same `agent_id` route to it. Carries no header — the
    /// header line for the group is sent separately via `Log`
    /// immediately before this message.
    RegisterAgent { agent_id: String },

    /// Prompt the user for y/n. Reply via the oneshot sender.
    Confirm { message: String, reply: oneshot::Sender<bool> },

    /// Suspend the TUI (leave raw mode) for the interactive work phase.
    Suspend,

    /// Resume the TUI (re-enter raw mode) after the interactive work
    /// phase. The receiver reconstructs the `Terminal` so the inline
    /// viewport re-anchors below whatever the child wrote.
    Resume,

    /// Terminate the TUI runtime loop. Sent on a clean shutdown path.
    Quit,
}
```

- `Progress` — the TUI shows only the latest one across all agents in
  the 1-row inline viewport (`active_progress` picks the
  most-recently-updated agent). Per-agent state is still tracked in
  `progress_groups` so events for inactive agents aren't lost; the
  viewport just renders one at a time.
- `Persist` — written to scrollback via `insert_styled`; rendered with
  the `Intent → Color` mapping in `ui::intent_color`. Always
  permanent, never overwritten.
- `AgentDone` — removes the agent's entry from `progress_groups`.
  Required so the live area shrinks when concurrent subagents finish.
- `RegisterAgent` — inserts an empty entry in `progress_groups`. The
  group's header is whatever `Log` line preceded this message; the
  TUI does not store the header itself.
- `Log` — written to scrollback via `insert_plain` (no styling). Used
  for phase headers, commit summaries, subagent completion lines, and
  warning banners. Splits on `\n` and clears all progress lines.
- `Confirm` — renders `▶ {message} [Y/n]` in the viewport until the
  user presses y/n/Enter; the answer goes back through the
  `oneshot::Sender`.
- `Suspend` / `Resume` — bracket the interactive work phase. `Resume`
  reconstructs the `Terminal` so the inline viewport's cached
  `viewport_area` doesn't clobber the child's last lines on the
  resumption clear.
- `Quit` — breaks out of the TUI runtime loop on shutdown. The
  receiver also exits when the channel closes (`None` from `recv`).

Ordering invariants the TUI relies on:

- `RegisterAgent` for an `agent_id` precedes the first `Progress` or
  `Persist` for that id. Progress for an unregistered agent is
  silently dropped (it won't appear in `progress_groups`).
- `AgentDone` ends the live-area lifetime; subsequent `Progress` for
  that id has no effect. Trailing `Persist` events are still inserted
  into scrollback because they don't depend on `progress_groups`.
- `Log` is the only message that clears *all* agents' progress —
  callers use it to fence phase boundaries.

## Agent Trait

```rust
pub type UISender = mpsc::UnboundedSender<UIMessage>;

#[async_trait]
pub trait Agent: Send + Sync {
    /// Interactive phase — agent owns the terminal.
    /// TUI must be suspended before calling this.
    async fn invoke_interactive(
        &self,
        prompt: &str,
        ctx: &PlanContext,
    ) -> Result<()>;

    /// Headless phase — agent streams events for the TUI to render.
    async fn invoke_headless(
        &self,
        prompt: &str,
        ctx: &PlanContext,
        phase: LlmPhase,
        tx: UISender,
    ) -> Result<()>;

    /// Dispatch a subagent to a target plan. Streams events with its
    /// own agent_id so concurrent subagents render as separate groups.
    async fn dispatch_subagent(
        &self,
        prompt: &str,
        target_plan: &str,
        tx: UISender,
    ) -> Result<()>;

    fn tokens(&self) -> HashMap<String, String>;

    async fn setup(&self, _ctx: &PlanContext) -> Result<()> {
        Ok(())
    }
}
```

Each headless invocation and each concurrent subagent gets a clone of
the same `UISender`. Agents only send `Progress`, `Persist`, and
`AgentDone` variants. The TUI holds the single receiver.

### Agent implementations

- **ClaudeCodeAgent** — spawns `claude` with
  `--output-format stream-json`, reads stdout line by line via
  `tokio::io::BufReader`, parses each line as JSON. `assistant` events
  with `tool_use` blocks become `Progress`; writes to highlight-matched
  paths (e.g. `memory.yaml`, `backlog.yaml`) and `result` events become
  `Persist`.
- **PiAgent** — spawns `pi` with `--mode json`. Different JSON event
  schema but the same output-event mapping.

Both agents share formatting logic (action marker parsing, highlight
rules, tool name cleaning) via the `format` module. Agent
implementations do not format strings themselves.

For `invoke_interactive`, both agents spawn their CLI with inherited
stdio (the TUI has been suspended, so the terminal is available).

## TUI Layout

Native terminal scrollback holds all permanent output; ratatui owns
only a 1-row inline viewport at the bottom for transient state. There
is no `Vec<String>` log buffer in the application — `Log` and
`Persist` lines are pushed into the terminal's scrollback via
`Terminal::insert_before` and the OS handles the rest.

```
… native terminal scrollback (unbounded, OS-managed) …
  ────────────────────────────────────────────────────
    ◆  REFLECT  ·  ravel-lite / core
    Distil session learnings into durable memory
  ────────────────────────────────────────────────────
    ★  Updating memory
    ADDED      New memory entry — description
    SHARPENED  Existing entry — what changed
    ⚙  COMMIT · reflect  ·  ravel-lite / core  ·  run-plan: reflect
  ▶ Dispatching 3 subagent(s)...
    ✓ sub-B-phase-cycle
─────────────────────────────────────────────────────  ← inline viewport (1 row)
      · Edit backlog.yaml (+1.4s)                     ← latest tool call OR confirm
```

### Scrollback (above the viewport)

Phase headers, commit summaries, `Persist` styled lines, subagent
completion messages, orchestrator warnings — everything permanent —
goes here via `insert_before`. Scrolling the terminal or `tee`ing
stderr to a file recovers the full history. The application does not
buffer any of this.

### Inline viewport (1 row, bottom)

`VIEWPORT_HEIGHT = 1`. The viewport answers a single question: "what's
running right now?" or "what am I waiting on?". It renders one of:

- **Confirm prompt** — `▶ {message} [Y/n]` in bold yellow, when
  `state.confirm_prompt` is set.
- **Latest tool call** — the most-recently-updated agent's
  `Progress` line (picked by `AppState::active_progress`, which
  ranks by `progress_at` timestamp), with a dim `(+Xs)` elapsed
  marker.
- **Blank** — when neither is active.

Per-agent state is tracked in `progress_groups`, but only ONE
agent's progress is rendered at a time because the viewport is
1 row. Concurrent subagents still update their entries; the viewport
just shows whichever was most recently active.

```rust
pub struct AppState {
    pub progress_groups: IndexMap<String, AgentProgress>,
    pub confirm_prompt: Option<ConfirmState>,
}

pub struct AgentProgress {
    pub progress: Option<StyledLine>,
    pub progress_at: Option<Instant>,
}
```

### No status bar

Earlier iterations carried a persistent status bar at the bottom of
the viewport. It was removed in favour of the linear-scrolling model
above: anything worth remembering is in scrollback, and the inline
viewport stays empty when the loop is idle.

## Phase Loop

The phase loop is an async function that drives the state machine. It
does not own the TUI — it communicates through a `UI` handle that
wraps the same `UISender` the agents use:

```rust
pub struct UI {
    tx: UISender,
}

impl UI {
    pub fn log(&self, text: &str) { ... }
    pub fn register_agent(&self, agent_id: &str, header: &str) { ... }
    pub async fn confirm(&self, message: &str) -> bool { ... }
    pub fn suspend(&self) { ... }
    pub fn resume(&self) { ... }
}
```

`confirm` sends a `Confirm` message carrying a `oneshot::Sender` and
awaits the reply. The TUI renders the prompt in the live area,
captures the keypress, and sends the response back.

### Phase state machine

```
Work → AnalyseWork → GitCommitWork → Reflect → GitCommitReflect
     → [if should_dream: Dream → GitCommitDream]
     → Triage → GitCommitTriage → Work → ...
```

Script phases (the `GitCommit*` steps) are handled inline — no
subprocess, just `git add` + `git commit` via `std::process::Command`.
The dream guard (`should_dream`) runs in the `GitCommitReflect`
handler; when dream is skipped, `GitCommitDream` is skipped too — a
`GitCommit*` phase only runs when its paired LLM phase produced work.

### Interactive phase handling

The work phase uses `invoke_interactive`, which needs the terminal:

1. `ui.suspend()` — TUI leaves raw mode, restores the normal terminal.
2. `invoke_interactive` runs — the agent subprocess inherits stdio.
3. `ui.resume()` — TUI re-enters raw mode and repaints from log
   history.

Ratatui repaints the full screen on resume, so the user sees the log
from before the interactive phase plus any new entries.

## Concurrent Subagent Dispatch

After the triage phase, the orchestrator reads
`<plan>/subagent-dispatch.yaml` (if present) and fans out one tokio
task per entry:

```rust
pub async fn dispatch_subagents(
    agent: Arc<dyn Agent>,
    plan_dir: &Path,
    ui: &UI,
) -> Result<()> {
    let dispatches = parse_dispatch_file(plan_dir)?;
    if dispatches.is_empty() { return Ok(()) }

    ui.log(&format!("\n▶ Dispatching {} subagent(s)...", dispatches.len()));

    let mut join_set: JoinSet<(String, Result<()>)> = JoinSet::new();

    for dispatch in &dispatches {
        let agent_id = basename(&dispatch.target);
        let tx = ui.sender();  // clone of the UISender

        ui.register_agent(
            &agent_id,
            &format!("  → {}: {}", dispatch.kind, dispatch.target),
        );

        let agent = Arc::clone(&agent);
        let prompt = build_prompt(dispatch);
        let target = dispatch.target.clone();
        let id = agent_id.clone();

        join_set.spawn(async move {
            let result = agent.dispatch_subagent(&prompt, &target, tx).await;
            (id, result)
        });
    }

    while let Some(Ok((agent_id, result))) = join_set.join_next().await {
        match result {
            Ok(()) => ui.log(&format!("  ✓ {}", agent_id)),
            Err(e) => ui.log(&format!("  ✗ {}: {}", agent_id, e)),
        }
    }

    fs::remove_file(plan_dir.join("subagent-dispatch.yaml"))?;
    Ok(())
}
```

Subagents all send on the same channel, distinguished by `agent_id`,
so the TUI renders them side-by-side as separate groups in the live
area.

## Formatting Module

Pure functions, no terminal writes, no mutable state:

```rust
pub struct FormattedOutput {
    pub text: String,
    pub persist: bool,
}

pub fn format_tool_call(tool: &ToolCall, phase: Option<LlmPhase>) -> FormattedOutput;
pub fn format_result_text(text: &str) -> String;
pub fn extract_edit_context(old: Option<&str>, new: Option<&str>) -> Option<String>;
pub fn clean_tool_name(name: &str) -> String;
```

Agent implementations call these to produce `FormattedOutput`, then map
to `UIMessage` variants. Phase highlight rules (`PHASE_HIGHLIGHTS`),
action marker styles (`ACTION_STYLES`), and phase info (`PHASE_INFO`)
are static data in this module.

Highlight deduplication (the `shown_highlights` set) lives in the
agent's headless invocation scope — reset per phase, checked before
emitting a `Persist` event.

## File Layout

### Source tree

```
ravel-lite/
├── Cargo.toml
├── defaults/                   # embedded by include_str!, written by init
│   ├── config.yaml
│   ├── agents/…                # includes agents/pi/subagents/ (pi subagent defs)
│   ├── phases/…
│   ├── fixed-memory/…
│   ├── ontology.yaml           # canonical edge-kind vocabulary
│   ├── create-plan.md          # prompt for `create` command
│   ├── survey.md               # prompt for `survey` (full snapshot)
│   ├── survey-incremental.md   # prompt for `survey` (delta path)
│   ├── discover-stage1.md      # prompt: per-project interaction surface
│   └── discover-stage2.md      # prompt: cross-project edge proposals
└── src/
    ├── main.rs                 # binary entry + CLI dispatch
    ├── lib.rs                  # library surface for integration tests
    ├── config.rs               # thin wrappers around the Lua resolver (config_lua)
    ├── config_lua.rs           # Lua-backed config layer (global + plan)
    ├── types.rs                # LlmPhase, ScriptPhase, PlanContext, etc.
    ├── agent/
    │   ├── mod.rs              # Agent trait
    │   ├── claude_code.rs      # ClaudeCodeAgent + stream parser
    │   ├── pi.rs               # PiAgent + stream parser
    │   ├── pty_capture.rs      # PTY transcript capture for debug logging
    │   └── common.rs           # shared helpers across agent impls
    ├── format.rs               # Pure formatting functions
    ├── phase_loop.rs           # Phase state machine (single cycle)
    ├── phase_summary.rs        # deterministic labelled-line phase summaries
    ├── multi_plan.rs           # Survey-driven multi-plan routing
    ├── subagent.rs             # Dispatch + concurrent execution
    ├── git.rs                  # git commit, baseline save, project-root math
    ├── dream.rs                # should_dream, update_baseline
    ├── debug_log.rs            # opt-in transcript log of LLM subprocess I/O
    ├── prompt.rs               # Template loading + token substitution
    ├── init.rs                 # `init` command — writes defaults + config.lua stub
    ├── create.rs               # `create` command — interactive plan scaffold
    ├── term_title.rs           # OSC-escape terminal title side channel
    ├── projects.rs             # ProjectsCatalog (name → path)
    ├── related_components.rs   # global name-indexed edge store
    ├── backlog_transitions.rs  # status-transition rules shared across verbs
    ├── migrate_v1_to_v2.rs     # one-shot config-file migration helper
    ├── survey.rs               # `survey` command entry
    ├── survey/
    │   ├── compose.rs          # prompt composition (fenced YAML blocks)
    │   ├── delta.rs            # incremental survey hash + merge
    │   ├── discover.rs         # survey-time proposal emission
    │   ├── invoke.rs           # spawn_claude_and_read helper
    │   ├── render.rs           # YAML → human-readable output
    │   └── schema.rs           # SurveyResponse, PlanRow, task counts
    ├── # NOTE: component-relationship ontology lives in the
    │   # `component-ontology` crate (atlas-contracts workspace).
    │   # `RelatedComponentsFile`, `EdgeKind`, `LifecycleScope`,
    │   # `EvidenceGrade`, the YAML loaders, and the kebab-case CLI
    │   # parsers all come from there; the host adapter lives in
    │   # `src/related_components.rs` below.
    ├── discover/               # two-stage LLM discovery of cross-project edges
    │   ├── stage1.rs           # per-project interaction-surface extraction
    │   ├── stage2.rs           # global edge-proposal fan-in
    │   ├── cache.rs            # (tree_sha, dirty_hash) keyed cache
    │   ├── tree_sha.rs         # git rev-parse + dirty-hash helpers
    │   ├── schema.rs           # SurfaceFile, SurfaceRecord, ProposalsFile
    │   └── apply.rs            # proposal → RelatedComponentsFile merge
    ├── state/                  # typed CRUD over plan-state files
    │   ├── phase.rs
    │   ├── migrate.rs
    │   ├── filenames.rs        # canonical state-file names shared by all verbs
    │   ├── backlog/            # schema + yaml_io + parse_md + verbs
    │   │                       # + render, lint_dependencies, repair_stale_statuses
    │   ├── memory/             # schema + yaml_io + parse_md + verbs
    │   ├── session_log/        # schema + yaml_io + parse_md + verbs
    │   └── discover_proposals/ # schema + verbs (add-proposal, load)
    └── ui.rs                   # Ratatui TUI, UI handle, rendering
```

Crate dependencies are defined in `Cargo.toml`.

### Config directory (created by `init`)

```
<config-dir>/
├── config.yaml                 # agent, headroom
├── config.local.yaml           # optional overlay; survives init --force
├── agents/
│   ├── claude-code/
│   │   ├── config.yaml         # per-phase model/param config
│   │   ├── config.local.yaml   # optional overlay; survives init --force
│   │   └── tokens.yaml         # template tokens
│   └── pi/
│       ├── config.yaml
│       ├── tokens.yaml
│       ├── prompts/
│       │   ├── system-prompt.md
│       │   └── memory-prompt.md
│       └── subagents/          # pi subagent definitions, deployed to
│           │                   # <project>/.pi/agents at setup time
│           ├── brainstorming.md
│           ├── tdd.md
│           └── writing-plans.md
├── phases/                     # phase prompt templates
│   ├── work.md
│   ├── analyse-work.md
│   ├── reflect.md
│   ├── dream.md
│   └── triage.md
├── fixed-memory/               # shared style guides
│   ├── coding-style.md
│   ├── coding-style-rust.md
│   └── memory-style.md
├── survey.md                   # prompt template for `survey` subcommand
└── create-plan.md              # prompt template for `create` subcommand
```

The config directory can live inside a project repo (versioned with
the code), shared across multiple projects, or kept standalone. Its
location is not tied to any project.

`init` skips files that already exist — rerunning it after an upgrade
picks up new defaults without overwriting user customisations.

### Plan directories

```
<project>/LLM_STATE/<plan>/
├── phase.md                  # current phase pointer (the only .md state file)
├── backlog.yaml
├── memory.yaml
├── session-log.yaml          # append-only history
├── latest-session.yaml       # most-recent session record (written by analyse-work)
├── related-plans.md          # optional prose-only cross-plan pointers
├── work-baseline             # SHA captured before the work phase
├── reflect-baseline          # SHA captured before reflect
├── triage-baseline           # SHA captured before triage
├── dream-baseline            # SHA captured before dream (when triggered)
├── dream-word-count          # memory word count at last dream baseline
└── …
```

The project root for a plan is derived from the plan path as
`<plan_dir>/../..` — pure path math, independent of where `.git`
lives. The conventional layout is `<project>/LLM_STATE/<plan>`, but
ravel-lite does not enforce the `LLM_STATE` segment; only
`<plan>/../..` is read as the project root, which makes the
orchestrator work cleanly inside a monorepo subtree as well as in a
single-repo layout (see "Monorepo subtrees" in `README.md`).

State files are typed YAML. The legacy `.md` shape (backlog.md,
memory.md, session-log.md, latest-session.md) is supported only via a
one-shot `ravel-lite state migrate` conversion; once migrated, the
prose-side mutators are the `ravel-lite state <kind> ...` verbs and
the `parse_md.rs` modules in `src/state/<kind>/` exist purely to read
the legacy form during migration.

## CLI and Invocation

The user interacts with `ravel-lite` directly once to create a
config directory, then drives the phase cycle with `ravel-lite run`.
There is no trampoline — the binary resolves its config directory via
an explicit precedence chain.

### `ravel-lite init <dir>`

Creates the config directory at `<dir>` with the default structure.
Default file contents are embedded in the binary at compile time via
`include_str!`.

After scaffolding, `init` prints guidance on how to make the binary
find that directory: either set `RAVEL_LITE_CONFIG=<dir>` or pass
`--config <dir>` on each invocation. When `<dir>` is the default
location (`dirs::config_dir()/ravel-lite/`), no setup is needed.

### Config discovery

Every `ravel-lite` subcommand that needs config resolves the config
directory in this order:

1. `--config <path>` CLI flag
2. `RAVEL_LITE_CONFIG` environment variable
3. Default location: `<dirs::config_dir()>/ravel-lite/`
4. Hard error with a pointer to `ravel-lite init`

No walk-up, no registry, no implicit project root. The first source
that resolves to an existing directory wins; if that directory doesn't
exist, `ravel-lite` errors with the candidate path and the source
that produced it.

### `ravel-lite run [--config <dir>] [--dangerous] [--survey-state <path>] <plan-dir> [<plan-dir> ...]`

Drives the phase loop. Takes an optional config root (resolved via
the discovery chain if omitted) and one or more plan directories.

With a **single plan directory** the loop runs continuously, prompting
between cycles. With **two or more plan directories**, multi-plan
mode kicks in: every cycle starts with a fresh survey across all
named plans, the user picks one from a numbered stdout prompt, and
one phase cycle runs for the chosen plan before the loop returns to
the survey prompt. `--survey-state <path>` is required in multi-plan
mode and rejected in single-plan mode; it is fed to the next survey
as `--prior` (the incremental-survey integration point) and
rewritten at the end of every survey, so the file is the persistent
integration point between cycles.

### `ravel-lite create [--config <dir>] <plan-dir>`

Interactively scaffolds a new plan directory. Validates that
`<plan-dir>` does not already exist and that its parent does, then
loads the prompt template at `<config-dir>/create-plan.md`, appends
the target path, and spawns a headful `claude` session with inherited
stdio. The user drives the conversation directly; Ravel-Lite's only job
is path validation, prompt composition, and post-hoc confirmation
that `phase.md` was created.

The session reuses the configured work-phase model
(`models.work` in the agent config) — plan creation is work-phase-like
reasoning and deserves the same budget. Claude is launched with
`--add-dir <parent>` to scope its tool access to the target parent
directory; interactive tool-approval prompts still fire as normal,
which is appropriate for a headful session. Supports `claude-code`
only in v1.

### `ravel-lite survey [--config <dir>] [--model <m>] <root> [<root> ...]`

Produces an LLM-driven multi-project plan-status overview. For each
plan root (a directory whose immediate subdirectories are plans),
discovers plans by `phase.md` presence and derives each plan's
project from the nearest ancestor directory containing `.git`. Bundles
each plan's `phase.md`, `backlog.md`, and `memory.md` into a single
fresh-context `claude` invocation alongside the prompt template at
`<config-dir>/survey.md`.

The **LLM emits YAML** matching a fixed schema (per-plan counts,
cross-plan blockers, parallel streams, recommended invocation
order); the tool parses the response and renders the final output
deterministically. The per-plan summary is grouped by project with a
compact `U/B/D/R` counts column per plan and notes rendered as a
wrapped body line below; the three prose sections (blockers, streams,
recommendations) use a header-then-body shape so wrap continuations
are never confused with new logical lines. This split means column
alignment, blank-line spacing, and wrap-width consistency are
guaranteed by the tool rather than hoped for from the LLM. Raw stdout
from claude is preserved in the error message when YAML parsing
fails, so malformed responses surface clearly.

Survey is single-shot and read-only by construction: no tool use, no
session persistence, no file writes. Model precedence is `--model`
flag → `models.survey` in the agent config → `DEFAULT_SURVEY_MODEL`
(currently `claude-haiku-4-5`). Supports `claude-code` only in v1.

### `ravel-lite survey-format <yaml-path>`

Render a saved YAML survey file (as produced by `ravel-lite survey`)
as human-readable markdown on stdout. Read-only; no network, no LLM
call. Useful for re-rendering an archived survey result without
re-running the LLM pass.

### `ravel-lite version`

Print the installed `ravel-lite` version. Equivalent to
`ravel-lite --version`; the subcommand form mirrors the rest of the
CLI surface. Output includes the crate version, the
`git describe --tags --always --dirty` of the source tree the binary
was built from, and the UTC build timestamp, so a running binary
always identifies the commit it came from.

### `ravel-lite state <subcommand>`

Mutate plan-state files from prompts without the per-call permission
overhead of letting the agent edit YAML directly. The intended
allowlist pattern is a single `Bash(ravel-lite state *)` entry that
covers every state mutation a phase prompt may issue.

Sub-commands:

- **`set-phase`** — rewrite `<plan-dir>/phase.md` to the given phase.
  Validates the phase string and requires `phase.md` to already
  exist.
- **`projects`** — manage the per-user projects catalog
  (`<config-dir>/projects.yaml`) that maps component names to
  absolute paths. Auto-populated on `ravel-lite run` when a new
  project is encountered. The shared component-relationship graph
  references components by name; this catalog is the per-user
  resolver.
- **`backlog`** — backlog CRUD verbs (`list`, `show`, `add`, `init`,
  `set-status`, `set-results`, `set-description`, `set-handoff`,
  `clear-handoff`, `set-title`, `set-dependencies`, `reorder`,
  `delete`, `lint-dependencies`, `repair-stale-statuses`). Every
  prompt-side mutation of `backlog.yaml` flows through one of these.
- **`memory`** — memory CRUD verbs. Dream rewrites `memory.yaml`
  per-entry through these rather than bulk-swapping the file.
- **`session-log`** — session-log verbs. `latest-session.yaml` is a
  single-record file written by analyse-work; `session-log.yaml` is
  the append-only history. `phase_loop::GitCommitWork` appends
  latest → log programmatically between phases.
- **`migrate`** — single-plan conversion of legacy `.md` state files
  into typed `.yaml` siblings. Covers `backlog.md`, `memory.md`,
  `session-log.md`, and `latest-session.md` (each written when
  present).
- **`related-components`** — global component-relationship graph at
  `<config-dir>/related-components.yaml`. Edges follow the
  component-ontology v2 schema (see `docs/component-ontology.md`);
  participants reference components by name (resolved per-user via
  the projects catalog), so the file is shareable between users.
- **`discover-proposals`** — Stage 2 discovery emits each edge
  through `add-proposal` rather than writing
  `discover-proposals.yaml` directly. A hallucinated `--kind` is
  rejected by clap with the full valid vocabulary in the error
  message, so the LLM retries that single call instead of corrupting
  the whole file.
- **`phase-summary`** — deterministic labelled-line summary of what
  changed in `backlog.yaml` (triage) or `memory.yaml`
  (reflect/dream) between a baseline commit and the current
  working-tree state. Replaces the LLM's manual re-transcription of
  its own tool calls at the end of each phase; the narrative
  preamble stays in the LLM.

Each verb is a thin Rust adapter over the typed CRUD modules under
`src/state/`. They validate inputs, write atomically, and emit
machine-readable confirmation on stdout.

### Configuration

The shipped defaults are authored as YAML inside the embedded set:

```yaml
# defaults/config.yaml
agent: claude-code
headroom: 1500
```

```yaml
# defaults/agents/claude-code/config.yaml
models:
  work: claude-opus-4-7
  analyse-work: claude-sonnet-4-6
  reflect: claude-sonnet-4-6
  dream: claude-sonnet-4-6
  triage: claude-sonnet-4-6
```

User overrides are expressed in **Lua**, not in YAML overlays. Two
optional layers compose on top of the embedded YAML defaults:

1. `<config-dir>/config.lua` — global override layer, scoped to this
   user / config root.
2. `<plan-dir>/config.lua` — per-plan override layer; runs in the same
   Lua state after the global layer.

Both files are evaluated by `config_lua::resolve` before the agent is
constructed. Setters are last-write-wins; `ravel.append_prompt`
accumulates across both layers in registration order. The Lua state is
not sandboxed — config is trusted code, same threat model as
`wezterm.lua` or Neovim's `init.lua`. A `config.lua` that throws is
surfaced as a normal Rust error tagged with the offending layer.

The Lua API is small and imperative:

```lua
-- <config-dir>/config.lua  (typical)
ravel.set_agent("claude-code")
ravel.set_model("work", "claude-opus-4-7")               -- active agent
ravel.set_model("reflect", "")                           -- defer to CLI default
ravel.set_model_for("pi", "work", "claude-opus-4-7")     -- specific agent
ravel.config.headroom = 2000
ravel.append_prompt("work", "Always run cargo fmt before declaring done.")
```

`ravel.config` is a writable table backed by `SharedConfig`'s top-level
fields (`agent`, `headroom`). `ravel.set_agent` swaps the active agent;
`ravel.set_model(phase, name)` updates the active agent's model;
`ravel.set_model_for(agent, phase, name)` targets a specific agent;
`ravel.set_headroom(n)` sets the dream-trigger headroom;
`ravel.set_token(agent, name, value)` overrides a single template
token; `ravel.append_prompt(phase, text)` accumulates appended prompt
fragments. Setting a model to the empty string suppresses the
`--model` flag at spawn time so the agent CLI uses its own default
(useful for machine-local pins that should follow whatever the editor
is currently configured for).

`init` writes a fully-commented `config.lua` stub to the config
directory; `init --force` preserves an existing `config.lua` rather
than overwriting it. The retired `*.local.yaml` overlay mechanism has
been removed — `migrate_v1_to_v2` provides a one-shot conversion
helper for users coming from older installs.

`ravel-lite run --dangerous <plan_dir>` is still supported as a
single-process knob: at startup it mutates the resolved `AgentConfig`
to set `dangerous: true` for every LLM phase before the agent is
constructed. For `claude-code`, that adds
`--dangerously-skip-permissions`; the flag is ignored with a warning
for other agents.

All `claude-code` invocations (interactive and headless) pass
`--add-dir <plan_dir>` so Claude's sandbox permits writes into the
plan directory. Without it, `memory.yaml`, `latest-session.yaml`,
`backlog.yaml`, and `subagent-dispatch.yaml` writes would be denied
because `plan_dir` lives outside the project tree (`current_dir` is
`project_dir`).
