# Raveloop — Rust/Ratatui Rewrite

> An orchestration loop for LLM development cycles.
> Compose. Reflect. Dream. Triage. Repeat.

Rewrite the raveloop orchestrator from TypeScript to Rust, replacing
manual ANSI cursor manipulation with a Ratatui TUI. Produces a single
static binary with zero runtime dependencies.

## Principles

- **No magic.** All config, prompts, phase state, and memory live as
  readable files on disk. The binary reads them at runtime. Nothing is
  embedded or compiled in.
- **Visible, auditable, adjustable.** Every input the orchestrator uses
  is a file the user can inspect and edit. Every state transition writes
  to the filesystem.
- **Agents are subprocesses.** The orchestrator spawns `claude` or `pi`
  CLI processes, reads their JSON stream output, and renders progress.
  It never calls LLM APIs directly.

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│  main()                                             │
│  Parse CLI args, load config, build PlanContext      │
│  Create agent, create TUI, run phase_loop           │
└──────────────────────┬──────────────────────────────┘
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
`mpsc::UnboundedSender<UIMessage>` channel. Both agents and the
phase loop send messages on the same channel.

```rust
pub enum UIMessage {
    // ── From agents ──────────────────────────────────────

    /// Overwritable progress for an agent (latest tool call).
    Progress { agent_id: String, text: String },

    /// Permanent output — appended to the scrolling log.
    Persist { agent_id: String, text: String },

    /// Agent finished. Remove its progress group from the live area.
    AgentDone { agent_id: String },

    // ── From the phase loop ──────────────────────────────

    /// Permanent log entry (phase headers, commit messages, etc.)
    Log(String),

    /// Register an agent group in the live area with a header line.
    RegisterAgent { agent_id: String, header: String },

    /// Update the status bar.
    SetStatus(StatusInfo),

    /// Prompt the user for y/n. Reply via the oneshot sender.
    Confirm { message: String, reply: oneshot::Sender<bool> },

    /// Suspend the TUI (leave raw mode) for interactive phase.
    Suspend,

    /// Resume the TUI (re-enter raw mode) after interactive phase.
    Resume,
}
```

- `Progress` — the TUI shows only the latest one per `agent_id`,
  indented under that agent's header in the live area.
- `Persist` — highlight labels (`★ Updating memory`), result text
  with action markers, tool errors. Appended to the log, never
  overwritten.
- `AgentDone` — the agent's progress group is removed from the
  live area.
- `RegisterAgent` — creates a group in the live area with a header.
  Subsequent `Progress` events for that `agent_id` appear indented
  under the header.
- `Log` — permanent output from the phase loop (not from an agent).
- `Suspend` / `Resume` — bracket the interactive work phase.

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

`UISender` is `tokio::sync::mpsc::UnboundedSender<UIMessage>`.
Each headless invocation and each concurrent subagent gets a clone
of the same sender. Agents only send `Progress`, `Persist`, and
`AgentDone` variants. The TUI holds the single receiver.

## Agent Implementations

### ClaudeCodeAgent

Spawns `claude` with `--output-format stream-json`. Reads stdout line
by line via `tokio::io::BufReader`. Parses each line as JSON. Maps
stream events to `OutputEvent`s using the same formatting logic
(ported from `format.ts`):

- `assistant` events with `tool_use` blocks → `Progress`
- Writes to highlight-matched paths (memory.md, backlog.md) → `Persist`
- `result` events → `Persist` (formatted with action marker recognition)
- On process exit → `Done`

For `invoke_interactive`: spawns `claude` with inherited stdio
(the TUI is suspended, so the terminal is available).

### PiAgent

Same pattern, spawns `pi` with `--mode json`. Different JSON event
schema but same `OutputEvent` mapping.

Both agents share the formatting logic (action marker parsing,
highlight rules, tool name cleaning). This lives in a `format` module,
not in the agent implementations.

## TUI Layout (Ratatui)

The terminal is divided into three regions using Ratatui's `Layout`:

```
┌──────────────────────────────────────────────────┐
│  Scrolling log (permanent output)                │ ← fills available space
│                                                  │
│  ────────────────────────────────────────────     │
│    ◆  REFLECT  ·  mnemosyne-orchestrator         │
│    Distil session learnings into durable memory   │
│  ────────────────────────────────────────────     │
│    ★  Updating memory                            │
│                                                  │
│    ADDED      New memory entry — description      │
│    SHARPENED  Existing entry — what changed       │
│                                                  │
│    ⚙  COMMIT · reflect  ·  run-plan: reflect     │
│                                                  │
│  ▶ Dispatching 3 subagent(s)...                  │
│    ✓ sub-B-phase-cycle                           │
├──────────────────────────────────────────────────┤
│  Live progress area (per-agent groups)           │ ← sized to content
│                                                  │
│    → child: sub-F-phase-cycle                    │
│        · Edit backlog.md                         │
│    → child: sub-H-phase-cycle                    │
│        · Read memory.md                          │
├──────────────────────────────────────────────────┤
│  Mnemosyne · mnemosyne-orchestrator · triage     │ ← 1 line, fixed
│  · claude-code                                   │
└──────────────────────────────────────────────────┘
```

### Log area (top)

A scrolling paragraph widget showing all permanent output. New entries
are appended at the bottom. Auto-scrolls to the latest entry. The log
is stored as a `Vec<String>`.

Phase headers, commit messages, persist events, subagent completion
messages, error banners — everything permanent goes here.

### Live progress area (middle)

Sized dynamically to fit the current number of active agents. Each
agent group has:
- A header line (e.g. `→ child: sub-F-phase-cycle`, or the phase
  header for the main agent)
- An indented progress line showing the latest tool call

When an agent completes (`Done` event), its group is removed and the
area shrinks. When no agents are active, this area is zero height.

State:

```rust
struct AgentProgress {
    header: String,
    progress: Option<String>,
}

// Keyed by agent_id, insertion-ordered
progress_groups: IndexMap<String, AgentProgress>,
```

### Status bar (bottom)

A single line showing the current state. Always visible (except during
interactive phase when the TUI is suspended).

```rust
struct StatusInfo {
    project: String,
    plan: String,
    phase: String,
    agent: String,
    cycle: Option<u32>,
}
```

Rendered as: `Mnemosyne · mnemosyne-orchestrator · reflect · claude-code`

Separated from the log area by a horizontal border.

## Phase Loop

The phase loop is an async function that drives the state machine.
It does not own the TUI — it communicates through a `UI` handle that
wraps the same `UISender` channel the agents use.

```rust
pub struct UI {
    tx: UISender,
}

impl UI {
    pub fn log(&self, text: &str) {
        let _ = self.tx.send(UIMessage::Log(text.to_string()));
    }
    pub fn register_agent(&self, agent_id: &str, header: &str) {
        let _ = self.tx.send(UIMessage::RegisterAgent { ... });
    }
    pub fn set_status(&self, status: StatusInfo) {
        let _ = self.tx.send(UIMessage::SetStatus(status));
    }
    pub async fn confirm(&self, message: &str) -> bool {
        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = self.tx.send(UIMessage::Confirm { message: ..., reply: reply_tx });
        reply_rx.await.unwrap_or(false)
    }
    pub fn suspend(&self) {
        let _ = self.tx.send(UIMessage::Suspend);
    }
    pub fn resume(&self) {
        let _ = self.tx.send(UIMessage::Resume);
    }
}
```

The `confirm` method sends a `Confirm` with a `oneshot::Sender` and
awaits the reply. The TUI renders the prompt in the live area,
captures the keypress, and sends the response back.

### Interactive phase handling

The work phase uses `invoke_interactive`, which needs the terminal.
Sequence:

1. `ui.suspend()` — TUI leaves raw mode, restores normal terminal
2. `invoke_interactive` runs — the agent subprocess inherits stdio
3. `ui.resume()` — TUI re-enters raw mode, repaints from log history

The log history is preserved in memory. Ratatui repaints the full
screen on resume, so the user sees the log from before the interactive
phase plus any new entries.

### Phase state machine

Same as the current TypeScript implementation:

```
Work → AnalyseWork → GitCommitWork → Reflect → GitCommitReflect
→ [Dream if shouldDream, else skip] → GitCommitDream
→ Triage → GitCommitTriage → Work → ...
```

Script phases (git commits) are handled inline — no subprocess, just
`git add` + `git commit` via `std::process::Command`.

The dream guard (`should_dream`) runs in the `GitCommitReflect`
handler, consistent with the fix we made earlier today.

## Concurrent Subagent Dispatch

After the triage phase, `dispatch_subagents` runs:

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

        // Register the agent group in the TUI
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

Each subagent runs as a separate tokio task. They send `Progress`,
`Persist`, and `AgentDone` messages through the same channel,
distinguished by `agent_id`. The TUI renders them as separate
groups in the live area.

## Formatting (ported from format.ts)

The formatting logic is ported to a `format` module with pure functions:

```rust
pub struct FormattedOutput {
    pub text: String,
    pub persist: bool,
}

pub fn format_tool_call(tool: &ToolCall, phase: Option<LlmPhase>) -> FormattedOutput { ... }
pub fn format_result_text(text: &str) -> String { ... }
pub fn extract_edit_context(old: Option<&str>, new: Option<&str>) -> Option<String> { ... }
pub fn clean_tool_name(name: &str) -> String { ... }
```

These are pure — no terminal writes, no state. The agent implementations
call them to produce `FormattedOutput`, then map to `OutputEvent`s.

Phase highlight rules (`PHASE_HIGHLIGHTS`), action marker styles
(`ACTION_STYLES`), and phase info (`PHASE_INFO`) are static data in
this module.

Highlight deduplication (the `shown_highlights` set) moves to the
agent's headless invocation scope — reset per phase, checked before
emitting a `Persist` event.

## File Layout

### Config directory (created by `init`, placed anywhere by the user)

See the `init` section above for the full tree. The config directory
can live inside a project repo (and be versioned with the code),
shared across multiple projects, or kept standalone. Its location
is not tied to any project.

### Plan directories (anywhere on disk)

Plan directories are passed as arguments to the `raveloop` trampoline.
They can live anywhere — inside a project repo, in a shared workspace,
or in a completely separate location. Each plan directory contains:

```
my-plan/
├── phase.md
├── backlog.md
├── memory.md
├── dream-baseline
├── session-log.md
├── related-plans.md
└── ...
```

The project directory for a given plan is found by walking up from
the plan directory to find `.git`.

### Rust source (the orchestrator binary)

```
raveloop/                 # separate repo for the binary
├── Cargo.toml
├── defaults/                # embedded by include_str!, written by init
│   ├── config.yaml
│   ├── agents/...
│   ├── phases/...
│   ├── fixed-memory/...
│   └── skills/...
└── src/
    ├── main.rs
    ├── config.rs            # YAML config loading
    ├── types.rs             # LlmPhase, ScriptPhase, PlanContext, etc.
    ├── agent/
    │   ├── mod.rs           # Agent trait
    │   ├── claude_code.rs   # ClaudeCodeAgent + stream parser
    │   └── pi.rs            # PiAgent + stream parser
    ├── format.rs            # Pure formatting functions
    ├── phase_loop.rs        # Phase state machine
    ├── subagent.rs          # Dispatch + concurrent execution
    ├── git.rs               # git commit, baseline save
    ├── dream.rs             # should_dream, update_baseline
    ├── prompt.rs            # Template loading + token substitution
    └── ui.rs                # Ratatui TUI, UI handle, rendering
```

## Crate Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
ratatui = "0.29"
crossterm = "0.28"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
indexmap = { version = "2", features = ["serde"] }
anyhow = "1"
async-trait = "0.1"
clap = { version = "4", features = ["derive"] }
```

## CLI and Invocation Model

The user never interacts with `raveloop-cli` directly except once,
to create a config directory. Day-to-day usage goes through a
trampoline shell script.

### `raveloop-cli init <dir>`

Creates a config directory at `<dir>` with the default structure
and a `raveloop` trampoline script:

```
<dir>/
├── raveloop                 # trampoline script (chmod +x)
├── config.yaml              # agent, headroom, phase params
├── agents/
│   ├── claude-code/
│   │   ├── config.yaml      # per-phase model/param config
│   │   └── tokens.yaml      # template tokens
│   └── pi/
│       ├── config.yaml
│       ├── tokens.yaml
│       └── prompts/
│           ├── system-prompt.md
│           └── memory-prompt.md
├── phases/                  # phase prompt templates
│   ├── work.md
│   ├── analyse-work.md
│   ├── reflect.md
│   ├── dream.md
│   └── triage.md
├── fixed-memory/            # shared style guides
│   ├── coding-style.md
│   ├── coding-style-rust.md
│   └── memory-style.md
└── skills/
    ├── brainstorming.md
    ├── tdd.md
    └── writing-plans.md
```

All default files are embedded in the binary at compile time via
`include_str!` and written to disk on `init`.

If a file already exists, `init` skips it (never overwrites). This
lets users run `init` after upgrading to pick up new defaults without
losing customisations.

The distinction: `init` uses embedded files to create the initial
structure. After that, the binary only reads from disk at runtime.
No embedded content is used during `run` — if a user deletes a
required file, the binary errors rather than silently falling back
to a built-in default.

### The `raveloop` trampoline

A generated shell script that lives in the config directory:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec raveloop-cli run --config "$SCRIPT_DIR" "$@"
```

Usage: `<dir>/raveloop <plan-directory>`

The trampoline always knows where the config is (its own directory).
The user can place the config directory anywhere — in a project repo,
shared across projects, or standalone. There is no magic directory
discovery.

### `raveloop-cli run --config <dir> <plan-directory>`

The main phase loop. Takes an explicit config root (provided by the
trampoline) and a plan directory.

The project directory is resolved by walking up from `<plan-directory>`
to find `.git`.

### Configuration drives everything

There are no CLI flags for agent selection, model choice, or
permissions. Everything comes from config:

```yaml
# config.yaml
agent: claude-code
headroom: 1500
```

```yaml
# agents/claude-code/config.yaml
models:
  work: claude-sonnet-4-6
  analyse-work: claude-haiku-4-5-20251001
  reflect: claude-haiku-4-5-20251001
  dream: claude-haiku-4-5-20251001
  triage: claude-haiku-4-5-20251001
params:
  work:
    dangerous: true
  analyse-work:
    dangerous: true
  reflect:
    dangerous: true
  dream:
    dangerous: true
  triage:
    dangerous: true
```

Per-phase `params` maps contain agent-specific CLI flags. For
claude-code, `dangerous: true` adds `--dangerously-skip-permissions`.
This keeps the agent interface generic — the orchestrator doesn't
need to know what flags each agent supports.

## What This Does NOT Change

- Prompt files — read at runtime, user-editable, no changes
- Config files — same YAML schema, same locations
- Phase state files (phase.md, memory.md, backlog.md, etc.) — same
  format, same locations, same semantics
- Git operations — same git add/commit logic
- Agent subprocess interaction — same CLI args, same stream protocols
- Dream guard logic — same word-count algorithm

The orchestrator is a new binary that reads the same files and spawns
the same subprocesses. From the perspective of the agent subprocesses
and the plan directory structure, nothing changes.

## Migration Path

1. Build the Rust binary (`raveloop-cli`) alongside the TypeScript source
2. Run `raveloop-cli init` to create a config directory from the
   existing files
3. Verify against existing plan directories (same phase transitions,
   same commits, same output content)
4. Replace `run-claude.sh` / `run-pi.sh` with the generated `raveloop`
   trampoline
5. Remove TypeScript source, package.json, tsconfig, vitest config
