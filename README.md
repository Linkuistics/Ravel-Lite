# LLM Context

Multi-agent orchestrator for backlog-driven LLM development cycles.
Supports both [Claude Code](https://claude.ai/code) and
[Pi](https://github.com/mariozechner/pi-coding-agent) as selectable
agent backends, with shared parameterized phase files and pluggable
configuration.

## Quick Start

```bash
npm install                    # install dependencies
npx tsx src/index.ts --agent claude-code <plan-directory>
npx tsx src/index.ts --agent pi <plan-directory>
```

The `--agent` flag selects the backend. If omitted, defaults to
the value in `config.yaml` (initially `claude-code`).

### Prerequisites

**Claude Code:**
- `claude` CLI installed and authenticated

**Pi:**
- `npm install -g @mariozechner/pi-coding-agent`
- `ANTHROPIC_API_KEY` set (or appropriate provider key)
- The orchestrator auto-installs the Pi subagent extension on first
  run — no manual Pi configuration needed

## Architecture

TypeScript orchestrator implementing a phase loop with an `Agent`
interface. Each agent backend is a class that handles invocation,
stream parsing, and prompt injection.

### Phase Chain

```
work → analyse-work → git-commit-work → [continue?] →
reflect → git-commit-reflect → [dream trigger?] →
dream → git-commit-dream →
triage → git-commit-triage → [continue?] → work
```

- **work** (interactive) — user steers task selection, implements task
- **analyse-work** (headless) — examines git diff to produce
  authoritative session log and commit message
- **reflect** (headless) — distils learnings into memory.md
- **dream** (headless, conditional) — lossless memory rewrite when
  word count exceeds baseline + headroom
- **triage** (headless) — adjusts backlog, emits subagent dispatch
  YAML for cross-plan propagation
- **git-commit-\*** — script-managed commits for per-phase audit trail
- **[continue?]** — user control points for clean exit

### Agent Differences

| Aspect | Claude Code | Pi |
|--------|------------|-----|
| Prompt injection | Relies on built-in (opaque) | Custom system-prompt.md + memory-prompt.md (version-controlled) |
| Subagents | Native Agent tool | Native subagent extension with auto-generated agent definitions |
| Tool names | Capitalized (Read, Write, Bash) | Lowercase (read, write, bash) |
| Thinking | Not configurable | Per-phase thinking levels |

Tool name differences are handled via `{{TOOL_*}}` tokens in shared
phase files, resolved per-agent from `agents/<name>/tokens.yaml`.

## Directory Layout

```
├── src/
│   ├── index.ts                    # CLI entry point
│   ├── phase-loop.ts               # Core phase cycle orchestrator
│   ├── prompt-composer.ts          # Load phase files, substitute tokens
│   ├── subagent-dispatch.ts        # Parse subagent-dispatch.yaml
│   ├── dream.ts                    # Dream trigger check
│   ├── git.ts                      # Per-phase git commit logic
│   ├── config.ts                   # Typed config loading
│   ├── types.ts                    # Enums and interfaces
│   └── agents/
│       ├── claude-code/            # ClaudeCodeAgent
│       └── pi/                     # PiAgent + auto-setup
│
├── phases/                         # Shared parameterized markdown
│   ├── work.md
│   ├── analyse-work.md
│   ├── reflect.md
│   ├── dream.md
│   └── triage.md
│
├── agents/                         # Agent-specific resources
│   ├── claude-code/
│   │   ├── config.yaml             # Model defaults
│   │   └── tokens.yaml             # Tool name mappings
│   └── pi/
│       ├── config.yaml             # Model/thinking/provider defaults
│       ├── tokens.yaml             # Tool name mappings
│       └── prompts/                # Injected into Pi sessions
│           ├── system-prompt.md
│           └── memory-prompt.md
│
├── skills/                         # Harness-independent skill content
│   ├── brainstorming.md
│   ├── writing-plans.md
│   └── tdd.md
│
├── fixed-memory/                   # Universal coding-style references
│   ├── coding-style.md
│   ├── coding-style-rust.md
│   └── memory-style.md
│
├── config.yaml                     # Shared config (headroom, default agent)
├── create-plan.md                  # How to create a new backlog plan
├── test/                           # vitest suite
└── package.json
```

## Configuration

### Shared (`config.yaml`)

```yaml
headroom: 1500          # Word-growth threshold for dream trigger
agent: claude-code      # Default agent (CLI --agent overrides)
```

### Per-Agent (`agents/<name>/config.yaml`)

```yaml
# Claude Code: model selection only
models:
  work: ""                        # harness default
  analyse-work: claude-sonnet-4-6
  reflect: claude-sonnet-4-6
  dream: claude-sonnet-4-6
  triage: claude-sonnet-4-6

# Pi: model + thinking + provider
provider: anthropic
models:
  work: claude-opus-4-6
  analyse-work: claude-sonnet-4-6
  ...
thinking:
  work: medium
  ...
```

### Token Substitution

Phase files use `{{PLACEHOLDER}}` tokens resolved at prompt
composition time:

| Token | Resolves to |
|-------|------------|
| `{{DEV_ROOT}}` | Parent of the project being worked on |
| `{{PROJECT}}` | Project root (.git directory) |
| `{{PLAN}}` | Plan directory path |
| `{{RELATED_PLANS}}` | Related plan paths block |
| `{{ORCHESTRATOR}}` | This project's root directory |
| `{{TOOL_READ}}` | `Read` or `read` (per agent) |
| `{{TOOL_WRITE}}` | `Write` or `write` |
| `{{TOOL_*}}` | Other tool name mappings |

## Skills

Skills are harness-independent markdown files in `skills/` with YAML
frontmatter. For Pi, the orchestrator auto-generates agent definitions
from these at startup (written to `{projectDir}/.pi/agents/`). For
Claude Code, the superpowers plugin provides equivalent functionality.

## Adding a New Agent

1. Create `agents/<name>/config.yaml` with model settings
2. Create `agents/<name>/tokens.yaml` with tool name mappings
3. Create `src/agents/<name>/index.ts` implementing the `Agent` interface
4. Add the agent case to `src/index.ts`

## Testing

```bash
npm test              # run vitest suite
npm run typecheck     # tsc --noEmit
```

## Backlog Plan Format

See `create-plan.md` for how to create plans. Plans live in
`PROJECT/LLM_STATE/{plan-name}/` with:

- `backlog.md` — mutable task list
- `memory.md` — distilled learnings
- `phase.md` — current phase
- `latest-session.md` — current session (overwritten each cycle)
- `session-log.md` — append-only audit trail
- `dream-baseline` — word count for dream trigger

## Origins

This project merges
[LLM_CONTEXT](https://github.com/Linkuistics/LLM_CONTEXT) (Claude Code)
and its Pi-harness fork into a single multi-agent orchestrator. The
original bash scripts have been rewritten in TypeScript.
