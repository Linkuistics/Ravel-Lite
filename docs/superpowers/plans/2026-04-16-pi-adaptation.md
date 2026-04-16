# LLM_CONTEXT_PI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the LLM_CONTEXT multi-session backlog-plan system from Claude Code to Pi (`@mariozechner/pi-coding-agent`), preserving every capability including cross-plan propagation, while pushing harness-specific assumptions into two named artifacts (`system-prompt.md` and the shell driver) so the result is maximally merge-able back into a harness-agnostic LLM_CONTEXT later.

**Architecture:**
1. **Explicit system-prompt file** (`system-prompt.md`) replaces the implicit CC system prompt — injected per-invocation via `pi --append-system-prompt "$(cat ...)"`.
2. **Externalized cross-plan propagation**: the triage phase writes `propagation.out.yaml` into the plan directory instead of dispatching in-session subagents. `run-plan.sh` reads the file after triage exits and fans out a fresh `pi` process per propagation target.
3. **Same four-phase cycle, same plan format, same fixed-memory/** as LLM_CONTEXT — anything not harness-specific is carried over verbatim to minimise fork surface.
4. **Ported auto-memory system** (`~/.claude-pi/projects/*/memory/`): mirrors Claude Code's auto-memory — user preferences, feedback corrections, project context, and external references persist across sessions and plans. Injected via system prompt; the work phase reads and writes memories, headless phases get read-only context.

**Tech Stack:** Bash (POSIX-ish, BSD-compatible awk), Pi CLI (`pi`), jq (for `--mode json` stream parsing), PyYAML-free YAML handling via a hand-rolled bash/awk parser for `propagation.out.yaml`, shellcheck for verification.

**Key reference facts gathered during research:**
- Pi's `--mode json` emits one JSON object per line, shape defined in `@mariozechner/pi-agent-core/dist/types.d.ts:284-322` (AgentEvent union) plus session-specific events in `pi-coding-agent/dist/core/agent-session.d.ts:40-65`.
- Relevant events: `tool_execution_start {toolCallId, toolName, args}`, `tool_execution_end {result, isError}`, `message_update {message, assistantMessageEvent}`, `message_end {message}`, `agent_end {messages}`.
- Pi text deltas live on `message_update`; the final assistant content is available on `message_end.message.content[]` where each content entry has `{type: "text", text: "..."}` (same shape as Anthropic's API).
- `--no-session` uses `SessionManager.inMemory()` (confirmed at `pi-coding-agent/dist/main.js:165`), so no session files accumulate across phase invocations.
- `pi "prompt"` seeds interactive mode with an initial prompt (help example: `pi "List all .ts files in src/"`), so the interactive work phase ports verbatim.
- `--append-system-prompt` accepts text or file contents as a direct argument, can be passed multiple times, and is additive on top of pi's built-in coding-agent prompt.
- Default provider is `google`; `--provider anthropic --model <id>` is needed to run on Anthropic models.

---

## File Structure

```
LLM_CONTEXT_PI/
├── .gitignore                         # NEW — ignore sessions, tee files
├── LICENSE                            # CP from LLM_CONTEXT
├── README.md                          # NEW — pi-specific user docs
├── create-plan.md                     # ADAPTED from LLM_CONTEXT
├── run-plan.sh                        # REWRITTEN for pi
├── config.sh                          # REWRITTEN for pi flags
├── system-prompt.md                   # NEW — explicit CC-implicit framing + auto-memory instructions
├── memory-prompt.md                   # NEW — memory system instructions (work-phase only, injected conditionally)
├── docs/
│   └── superpowers/plans/2026-04-16-pi-adaptation.md  # this file
├── phases/
│   ├── work.md                        # ADAPTED (tool names, path rule removed)
│   ├── reflect.md                     # CP from LLM_CONTEXT (unchanged)
│   ├── compact.md                     # CP from LLM_CONTEXT (unchanged)
│   └── triage.md                      # REWRITTEN — no subagent dispatch
├── fixed-memory/
│   ├── coding-style.md                # CP from LLM_CONTEXT (unchanged)
│   ├── coding-style-rust.md           # CP from LLM_CONTEXT (unchanged)
│   └── memory-style.md                # CP from LLM_CONTEXT (unchanged)
└── test/
    ├── fixtures/
    │   ├── pi-stream-sample.jsonl     # hand-crafted pi event fixture
    │   ├── propagation-sample.yaml    # hand-crafted propagation fixture
    │   └── pi-stream-empty.jsonl      # empty-run fixture (no tool calls)
    ├── test-format-pi-stream.sh       # unit test for the stream formatter
    ├── test-parse-propagation.sh      # unit test for the yaml parser
    └── test-compose-prompt.sh         # unit test for placeholder substitution
```

**Split rationale:**
- `system-prompt.md` is a new artifact class — deliberately named, deliberately auditable, deliberately separate from the phase prompts so future work can rewrite either independently.
- `phases/*.md` stay per-phase so each can be read by its phase without loading the others, mirroring LLM_CONTEXT.
- `test/` is new because LLM_CONTEXT has no test suite. The three tests cover the three pure bash functions that deserve unit-level confidence: the stream formatter, the propagation parser, and the prompt composer.

**Refactor note:** `run-plan.sh` will be written with its main loop guarded by `if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then ... fi`, so test files can source the script and call individual functions without triggering the main loop. This differs slightly from LLM_CONTEXT's `run-plan.sh` and is a small improvement worth keeping.

---

## Task 0: Initialise the repo

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/.gitignore`

- [ ] **Step 0.1: Initialise git**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git init -b main
```

- [ ] **Step 0.2: Write `.gitignore`**

```
# Pi session storage (tests may use ephemeral local dirs)
.pi/
*.session.jsonl
# Test tee files
test/tmp/
# OS
.DS_Store
```

- [ ] **Step 0.3: Initial empty commit**

```bash
git add .gitignore
git commit -m "Initial commit: gitignore only"
```

---

## Task 1: Copy verbatim content from LLM_CONTEXT

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/LICENSE`
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/fixed-memory/coding-style.md`
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/fixed-memory/coding-style-rust.md`
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/fixed-memory/memory-style.md`
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/phases/reflect.md`
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/phases/compact.md`

These files contain no harness-specific content; they copy byte-for-byte.

- [ ] **Step 1.1: Copy LICENSE**

```bash
cp /Users/antony/Development/LLM_CONTEXT/LICENSE \
   /Users/antony/Development/LLM_CONTEXT_PI/LICENSE
```

- [ ] **Step 1.2: Copy fixed-memory/**

```bash
mkdir -p /Users/antony/Development/LLM_CONTEXT_PI/fixed-memory
cp /Users/antony/Development/LLM_CONTEXT/fixed-memory/coding-style.md \
   /Users/antony/Development/LLM_CONTEXT/fixed-memory/coding-style-rust.md \
   /Users/antony/Development/LLM_CONTEXT/fixed-memory/memory-style.md \
   /Users/antony/Development/LLM_CONTEXT_PI/fixed-memory/
```

- [ ] **Step 1.3: Copy reflect.md and compact.md verbatim**

```bash
mkdir -p /Users/antony/Development/LLM_CONTEXT_PI/phases
cp /Users/antony/Development/LLM_CONTEXT/phases/reflect.md \
   /Users/antony/Development/LLM_CONTEXT/phases/compact.md \
   /Users/antony/Development/LLM_CONTEXT_PI/phases/
```

- [ ] **Step 1.4: Verify byte-for-byte identity**

```bash
diff -rq /Users/antony/Development/LLM_CONTEXT/fixed-memory \
         /Users/antony/Development/LLM_CONTEXT_PI/fixed-memory
diff /Users/antony/Development/LLM_CONTEXT/phases/reflect.md \
     /Users/antony/Development/LLM_CONTEXT_PI/phases/reflect.md
diff /Users/antony/Development/LLM_CONTEXT/phases/compact.md \
     /Users/antony/Development/LLM_CONTEXT_PI/phases/compact.md
```

Expected: `diff -rq` prints nothing. Both `diff` commands print nothing. Exit code 0.

- [ ] **Step 1.5: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add LICENSE fixed-memory/ phases/reflect.md phases/compact.md
git commit -m "Port verbatim content: license, fixed-memory, reflect, compact"
```

---

## Task 2: Write the explicit system prompt artifact

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/system-prompt.md`

This file encodes everything Claude Code injects implicitly that the phase prompts rely on. Each section has a one-line reason for existing. The driver will pass this file's content via `--append-system-prompt` on every pi invocation (interactive and headless).

- [ ] **Step 2.1: Write `system-prompt.md`**

```markdown
# LLM_CONTEXT_PI System Prompt Addendum

You are running a single phase of a multi-session backlog plan under
LLM_CONTEXT_PI. Everything in this file is invariant context — it does
not depend on which phase is running or which plan is active. The
phase-specific prompt follows this addendum.

## Fresh-context mandate

Each phase starts with a fresh conversation. You have no memory of
previous phases or sessions. Read only what the phase prompt tells you
to read, in the order it specifies. Do not try to infer prior context
from file modification times, git history, or guesswork.

## Tool etiquette

Prefer the dedicated tools over shell equivalents:

- `read` for file contents, not `bash cat`
- `grep` for content search, not `bash grep` or `bash rg`
- `find` for file discovery, not `bash find` or `bash ls`
- `ls` for directory listing, not `bash ls`
- `edit` / `write` for file mutation, not `bash` heredocs or `sed`/`awk`

Use `bash` only for things the other tools cannot do: running tests,
compilers, build systems, formatters, or other project-specific
commands. If you find yourself reaching for `bash cat file.txt`, use
`read` instead.

## Path placeholder rule

Any file you read inside this project may contain literal
`{{PROJECT}}`, `{{DEV_ROOT}}`, or `{{PLAN}}` placeholder tokens. These
are substitution tokens used by the LLM_CONTEXT_PI driver. Substitute
them mentally with the absolute paths from the phase prompt before
passing a path to any tool. Never pass a literal `{{...}}` string to
`read`, `bash`, or any other tool.

## Verification-before-completion

Never mark a task done, a fix applied, or a phase complete without
evidence. Run the tests, inspect the output, check the state. If you
cannot verify a change (for example, UI work in a headless phase),
state so explicitly in your output — do not claim success.

## Negative file-read discipline

If a phase prompt explicitly tells you NOT to read a file, do not read
it, even if your instincts suggest it would help. Several phases have
load-bearing negative reads — reading the forbidden file would pollute
the fresh context that the phase depends on.

## Destructive operation discipline

Do not run destructive or irreversible operations without checking
first:

- `git reset --hard`, `git push --force`, `git branch -D`,
  `git checkout .`, `git clean -f`
- `rm -rf` on anything outside a temporary scratch directory
- Dropping, truncating, or rewriting database tables
- Any operation that overwrites uncommitted changes

These are not blocked — use them when genuinely needed — but they
warrant an extra beat of thought, and usually a sentence in your
output acknowledging what you are about to do.

## Tone

Be concise. Report results, not intentions. Do not narrate your
internal deliberation. When an operation completes, state the
outcome in one line; elaborate only if there is an actionable
surprise.
```

- [ ] **Step 2.2: Sanity-check the file**

```bash
wc -l /Users/antony/Development/LLM_CONTEXT_PI/system-prompt.md
```

Expected: ~60 lines. File exists.

- [ ] **Step 2.3: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add system-prompt.md
git commit -m "Add explicit system-prompt.md replacing CC-implicit framing"
```

---

## Task 2B: Write the auto-memory system

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/memory-prompt.md`

This file contains the memory system instructions — ported from Claude Code's auto-memory system prompt. It is injected via `--append-system-prompt` only during the **work phase** (interactive, where user corrections and preferences emerge). Headless phases receive the memory *index* (MEMORY.md contents) as read-only context but not the full write instructions.

The memory directory lives at `~/.claude-pi/projects/<path-encoded>/memory/` where `<path-encoded>` is the absolute project path with `/` replaced by `-`. This mirrors CC's `~/.claude/projects/` convention.

`run-plan.sh` handles:
- Computing the memory directory path from `$PROJECT`
- Creating the directory if absent
- Reading `MEMORY.md` if present
- Injecting memory-prompt.md + MEMORY.md contents during work phase
- Injecting MEMORY.md contents only (read-only context) during headless phases
- Substituting `{{MEMORY_DIR}}` in memory-prompt.md

- [ ] **Step 2B.1: Write `memory-prompt.md`**

```markdown
# Auto Memory System

You have a persistent, file-based memory system at `{{MEMORY_DIR}}`.
This directory already exists — write to it directly with the `write`
tool (do not run `bash mkdir` or check for its existence).

Build up this memory over time so that future sessions have a complete
picture of who the user is, how they like to collaborate, what
behaviors to avoid or repeat, and the context behind the work.

If the user explicitly asks you to remember something, save it
immediately. If they ask you to forget something, find and remove the
relevant entry.

## Memory types

### user
Information about the user's role, goals, responsibilities, and
knowledge. Helps tailor future behavior.

**When to save:** When you learn details about the user's role,
preferences, responsibilities, or expertise.

### feedback
Guidance the user has given about how to approach work — corrections
AND confirmations. Record from failure AND success.

**When to save:** When the user corrects your approach ("no not that",
"don't", "stop doing X") OR confirms a non-obvious approach worked
("yes exactly", "perfect, keep doing that").

**Structure:** Lead with the rule, then a **Why:** line and a
**How to apply:** line.

### project
Information about ongoing work, goals, initiatives, bugs, or
incidents not derivable from code or git history.

**When to save:** When you learn who is doing what, why, or by when.
Convert relative dates to absolute dates when saving.

**Structure:** Lead with the fact/decision, then **Why:** and
**How to apply:** lines.

### reference
Pointers to where information lives in external systems.

**When to save:** When you learn about external resources and their
purpose.

## What NOT to save

- Code patterns, architecture, file paths — derive from the codebase.
- Git history, recent changes — use `git log` / `git blame`.
- Debugging solutions — the fix is in the code.
- Anything in AGENTS.md or CLAUDE.md files.
- Ephemeral task details or current conversation context.

## How to save

**Step 1** — write the memory to its own file in `{{MEMORY_DIR}}`
(e.g., `user_role.md`, `feedback_testing.md`) using this format:

```markdown
---
name: {{memory name}}
description: {{one-line description}}
type: {{user, feedback, project, reference}}
---

{{memory content}}
```

**Step 2** — add a pointer to that file in `{{MEMORY_DIR}}/MEMORY.md`.
Each entry should be one line, under ~150 characters:
`- [Title](file.md) — one-line hook`. MEMORY.md has no frontmatter.

## When to access memories
- When memories seem relevant, or the user references prior work.
- You MUST access memory when the user explicitly asks to recall.
- If the user says to ignore memory: do not apply or mention it.
- Memory records can become stale. Verify against current state before
  acting on recalled information. If a memory conflicts with current
  state, trust what you observe now and update or remove the stale
  memory.

## Before recommending from memory

A memory that names a file, function, or flag is a claim about when
it was written. It may have been renamed, removed, or never merged.
Before recommending: check it still exists. "The memory says X exists"
is not "X exists now."
```

- [ ] **Step 2B.2: Sanity-check the file**

```bash
wc -l /Users/antony/Development/LLM_CONTEXT_PI/memory-prompt.md
```

Expected: ~90 lines. File exists.

- [ ] **Step 2B.3: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add memory-prompt.md
git commit -m "Add memory-prompt.md: auto-memory system instructions for work phase"
```

---

## Task 3: Adapt `phases/work.md`

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/phases/work.md`

Differences from `LLM_CONTEXT/phases/work.md`:
1. Drop the inline path-placeholder rule — now in `system-prompt.md`.
2. Change `{{DEV_ROOT}}/LLM_CONTEXT/fixed-memory/` references to `{{DEV_ROOT}}/LLM_CONTEXT_PI/fixed-memory/`.
3. Nothing else — tool names (Read / read, Edit / edit) are already mentioned via prose only, so the same text works.

- [ ] **Step 3.1: Start from the LLM_CONTEXT copy**

```bash
cp /Users/antony/Development/LLM_CONTEXT/phases/work.md \
   /Users/antony/Development/LLM_CONTEXT_PI/phases/work.md
```

- [ ] **Step 3.2: Replace `LLM_CONTEXT/fixed-memory` with `LLM_CONTEXT_PI/fixed-memory`**

Use Edit, not sed — we need a single controlled substitution.

Replace `{{DEV_ROOT}}/LLM_CONTEXT/fixed-memory/` with `{{DEV_ROOT}}/LLM_CONTEXT_PI/fixed-memory/` (three occurrences in work.md).

- [ ] **Step 3.3: Remove the inline placeholder-note section**

Delete the paragraph that begins "**Placeholder note:** any file you Read inside this project" down through "absolute paths from this prompt before passing the path to the Read tool." Replace the surrounding text so the "Related plans" heading follows directly after the "Required reads" list.

The final reads-block in `phases/work.md` should be:

```markdown
## Required reads

Read the following files in order:

1. `{{PROJECT}}/README.md` — project conventions, architecture, build/test
   commands, and gotchas.
2. `{{PLAN}}/backlog.md` — the current task backlog
3. `{{PLAN}}/memory.md` — distilled learnings from prior sessions
4. `{{PLAN}}/related-plans.md` — declared peer-project relationships
   (only if the file exists)

## Related plans
```

- [ ] **Step 3.4: Verify no Claude-specific tool names remain**

```bash
grep -nE "\b(Read|Write|Edit|MultiEdit|Glob|Grep|Bash|Task|TodoWrite|WebFetch|WebSearch)\b" \
   /Users/antony/Development/LLM_CONTEXT_PI/phases/work.md || echo "clean"
```

Expected: prints `clean`. The file mentions tools in prose (e.g., "Read tool") — after this task those specific capitalised names should be replaced with lowercase pi tool names (`read`, `write`, etc.) or rephrased. Use Edit to fix any hits the grep reports.

- [ ] **Step 3.5: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add phases/work.md
git commit -m "Adapt work.md: drop inline placeholder rule, retarget fixed-memory"
```

---

## Task 4: Rewrite `phases/triage.md` — externalized propagation

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/phases/triage.md`

This is the largest prompt change. Triage no longer dispatches subagents. It writes a structured propagation list to `{{PLAN}}/propagation.out.yaml`, and the driver handles fan-out.

- [ ] **Step 4.1: Write `phases/triage.md`**

```markdown
You are running the TRIAGE phase of a multi-session backlog plan. The
triage phase runs headlessly at the end of each cycle. Its job is to
review and adjust the task backlog based on what the cycle learned, and
to emit a structured list of cross-plan propagations for the driver to
dispatch after this phase exits.

## Required reads

1. `{{PLAN}}/backlog.md` — the task backlog
2. `{{PLAN}}/memory.md` — distilled learnings

## Related plans

{{RELATED_PLANS}}

## Do NOT read

- `{{PLAN}}/session-log.md`
- `{{PLAN}}/latest-session.md`
- **Any file under a sibling, parent, or child plan directory.**
  Cross-plan awareness comes from the Related plans block above (paths
  only). You do not read foreign plan content from this phase — the
  driver dispatches fresh pi processes per propagation target after
  you exit, and each of those processes reads its own target's
  backlog/memory with fresh context.

## Behavior

### 1. Local triage

Review each task in `backlog.md`:

- Still relevant?
- Priority changed?
- Needs splitting?

Add new tasks implied by learnings in `memory.md`. **Delete completed
tasks.** Remove any task with status `done`, and clear any "Completed
Tasks" section entirely — heading and all. Reflect has already run and
anything worth keeping is now in `memory.md`; the session-log entry is
the durable record of what happened. The backlog is for work that
still needs doing, and must never carry a standing "Completed" holding
area between cycles.

Remove tasks that are no longer relevant (dependencies met, approach
changed, out of scope). Reprioritize based on what the cycle revealed.

**Scan task descriptions for embedded blockers.** A spike, validation
step, or shared dependency buried inside one task's description is
invisible to future work phases until that task runs — even when it
could run in parallel today. Promote any such blocker to its own
top-level task so it surfaces as executable work.

### 2. Cross-plan propagation — emit a structured list

Look at the Related plans block above. For each listed plan (siblings,
parents, children), judge whether this session's learnings affect that
plan. Rules of thumb:

- **Children:** if the learning changes how downstream consumers should
  use this plan's outputs, it affects children.
- **Parents:** if the learning reveals a bug, limitation, or gap in
  something parents produce, it affects parents.
- **Siblings:** if the learning generalizes to a shared pattern across
  siblings, it affects siblings.

**Write** `{{PLAN}}/propagation.out.yaml` containing one entry per
related plan that warrants propagation. Use this exact format:

```yaml
propagations:
  - target: /absolute/path/to/related/plan
    kind: child         # or "parent" or "sibling"
    summary: |
      One to three paragraphs describing the learning and why it
      affects this target. The receiving pi process will be given
      this summary plus the target path, and told to read the
      target's backlog.md and memory.md and apply whatever updates
      are warranted.
```

Rules:
- Use absolute paths (the Related plans block already shows them).
- Use `|` (block scalar) for `summary` so multi-line text works.
- Omit the whole file if there are no propagations. An empty or absent
  `propagation.out.yaml` tells the driver there is nothing to fan out.
- Do **not** attempt to dispatch anything yourself. You do not have a
  subagent tool and should not try to invoke one. The driver reads
  `propagation.out.yaml` after you exit and handles dispatch.

### 3. Finishing

Write `work` to `{{PLAN}}/phase.md`. Stop.
```

- [ ] **Step 4.2: Verify the file mentions neither `Task` nor `subagent` in dispatch contexts**

```bash
grep -nE "\b(Task|subagent|dispatch)\b" \
   /Users/antony/Development/LLM_CONTEXT_PI/phases/triage.md
```

Expected: matches only the words "dispatch" and "subagent" in the
explanatory paragraph that tells the model NOT to dispatch. No matches
on capitalised "Task" (the CC tool name).

- [ ] **Step 4.3: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add phases/triage.md
git commit -m "Rewrite triage.md: externalize cross-plan propagation via yaml"
```

---

## Task 5: Write `config.sh` for pi

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/config.sh`

- [ ] **Step 5.1: Write `config.sh`**

```bash
# Configuration for run-plan.sh. Sourced at startup.
#
# Edit this file to override the defaults. Keep variable assignments in
# plain `KEY=value` form — the file is sourced as bash, so it can also
# contain comments (lines starting with `#`) and inline expansions if
# needed.

# -----------------------------------------------------------------------------
# Pi provider and per-phase model selection
# -----------------------------------------------------------------------------
#
# Pi is multi-provider. The provider determines which environment
# variable supplies the API key and which default model namespace
# applies. Each phase below sets a model pattern that pi will match
# against available models (see `pi --list-models`).
#
# Set PHASE_MODEL to the empty string to let pi pick its default (the
# per-settings provider default). The headless trio (reflect, compact,
# triage) defaults to Sonnet 4.6 because those phases are editorial /
# judgment-light and Opus is overkill there. WORK_MODEL is set to Opus
# 4.6 because the work phase is where the engineering happens.
#
# Why not pick one model everywhere:
# - Reflect applies a style guide to a small set of new learnings.
# - Compact rewrites memory.md prose under a strict-lossless contract.
# - Triage adjusts task ordering and emits a propagation list.
# All three are editorial and Sonnet handles them well. Opus is worth
# it for the work phase where model quality moves actual outcomes.

PROVIDER="anthropic"

WORK_MODEL="claude-opus-4-6"
REFLECT_MODEL="claude-sonnet-4-6"
COMPACT_MODEL="claude-sonnet-4-6"
TRIAGE_MODEL="claude-sonnet-4-6"

# -----------------------------------------------------------------------------
# Pi thinking level per phase
# -----------------------------------------------------------------------------
#
# Pi supports thinking levels: off, minimal, low, medium, high, xhigh.
# Setting a thinking level costs tokens and latency but improves
# accuracy on hard tasks. The editorial phases do not need it; the
# work phase benefits from it on complex tasks.
#
# Set to empty string to let pi / the model decide.

WORK_THINKING="medium"
REFLECT_THINKING=""
COMPACT_THINKING=""
TRIAGE_THINKING=""

# -----------------------------------------------------------------------------
# Compaction trigger
# -----------------------------------------------------------------------------
#
# memory.md must grow this many words past `<plan>/compact-baseline`
# before the compact phase fires. Lower = more frequent compaction,
# higher = less. 1500 words is roughly 7–8 cycles at observed growth
# rates. Relative threshold tracks unreflected growth rather than
# absolute size.

HEADROOM=1500
```

- [ ] **Step 5.2: Shellcheck**

```bash
shellcheck /Users/antony/Development/LLM_CONTEXT_PI/config.sh
```

Expected: no output, exit code 0. If shellcheck is not installed, skip
the check — it is a nice-to-have, not a requirement.

- [ ] **Step 5.3: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add config.sh
git commit -m "Add config.sh: per-phase pi provider, model, thinking level"
```

---

## Task 6: Write the stream-formatter fixture

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/test/fixtures/pi-stream-sample.jsonl`
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/test/fixtures/pi-stream-empty.jsonl`

These are hand-crafted fixtures mirroring real pi `--mode json` output. They drive the test for `format_pi_stream()` in Task 8. Fixtures come *first* so the parser is tested against realistic data before it is written (TDD discipline, shell-adapted).

- [ ] **Step 6.1: Write `pi-stream-sample.jsonl`**

```jsonl
{"type":"agent_start"}
{"type":"turn_start"}
{"type":"tool_execution_start","toolCallId":"tc_1","toolName":"read","args":{"path":"/Users/x/Development/Proj/LLM_STATE/plan/backlog.md"}}
{"type":"tool_execution_end","toolCallId":"tc_1","toolName":"read","result":"# Backlog...","isError":false}
{"type":"tool_execution_start","toolCallId":"tc_2","toolName":"grep","args":{"pattern":"TODO","path":"src/"}}
{"type":"tool_execution_end","toolCallId":"tc_2","toolName":"grep","result":"src/lib.ts:42:// TODO","isError":false}
{"type":"tool_execution_start","toolCallId":"tc_3","toolName":"bash","args":{"command":"cargo test --workspace"}}
{"type":"tool_execution_end","toolCallId":"tc_3","toolName":"bash","result":"test result: ok. 42 passed","isError":false}
{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Triage complete. Wrote 2 propagations to propagation.out.yaml.\n"}],"stopReason":"end_turn"}}
{"type":"turn_end","message":{"role":"assistant"},"toolResults":[]}
{"type":"agent_end","messages":[]}
```

- [ ] **Step 6.2: Write `pi-stream-empty.jsonl`**

```jsonl
{"type":"agent_start"}
{"type":"turn_start"}
{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Nothing to do.\n"}],"stopReason":"end_turn"}}
{"type":"turn_end","message":{"role":"assistant"},"toolResults":[]}
{"type":"agent_end","messages":[]}
```

- [ ] **Step 6.3: Verify valid JSONL**

```bash
jq -c . < /Users/antony/Development/LLM_CONTEXT_PI/test/fixtures/pi-stream-sample.jsonl > /dev/null && echo ok
jq -c . < /Users/antony/Development/LLM_CONTEXT_PI/test/fixtures/pi-stream-empty.jsonl > /dev/null && echo ok
```

Expected: two `ok` lines.

- [ ] **Step 6.4: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add test/fixtures/pi-stream-sample.jsonl test/fixtures/pi-stream-empty.jsonl
git commit -m "Add pi stream fixtures for format_pi_stream tests"
```

---

## Task 7: Write the stream-formatter test

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh`

- [ ] **Step 7.1: Write the test script**

```bash
#!/usr/bin/env bash
# Test for format_pi_stream() in run-plan.sh.
#
# Sources run-plan.sh without running its main loop (main loop is
# guarded on BASH_SOURCE == $0). Pipes the fixture through
# format_pi_stream and asserts the output contains expected lines.

set -eu

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"

# shellcheck source=../run-plan.sh
. "$ROOT/run-plan.sh"

pass=0
fail=0

assert_contains() {
    local needle="$1"
    local haystack="$2"
    local label="$3"
    if printf '%s' "$haystack" | grep -qF "$needle"; then
        printf 'PASS %s\n' "$label"
        pass=$((pass + 1))
    else
        printf 'FAIL %s — expected to contain: %s\n' "$label" "$needle"
        fail=$((fail + 1))
    fi
}

out="$(format_pi_stream < "$HERE/fixtures/pi-stream-sample.jsonl")"

assert_contains "→ read" "$out" "read tool line"
assert_contains "backlog.md" "$out" "read path rendered"
assert_contains "→ grep" "$out" "grep tool line"
assert_contains "/TODO/" "$out" "grep pattern rendered"
assert_contains "→ bash" "$out" "bash tool line"
assert_contains "cargo test --workspace" "$out" "bash command rendered"
assert_contains "Triage complete" "$out" "final assistant text rendered"
assert_contains "propagation.out.yaml" "$out" "final text full"

out_empty="$(format_pi_stream < "$HERE/fixtures/pi-stream-empty.jsonl")"
assert_contains "Nothing to do" "$out_empty" "empty fixture assistant text"

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
```

- [ ] **Step 7.2: Make it executable**

```bash
chmod +x /Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh
```

- [ ] **Step 7.3: Run the test — it should FAIL because run-plan.sh does not yet exist**

```bash
/Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh
```

Expected: exit non-zero, likely with "No such file or directory" on the `. "$ROOT/run-plan.sh"` line. This is the failing-test state.

- [ ] **Step 7.4: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add test/test-format-pi-stream.sh
git commit -m "Add failing test for format_pi_stream"
```

---

## Task 8: Write `run-plan.sh` skeleton with `format_pi_stream()`

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh`

Start with arg parsing, self-location, project-root walk-up, the source of `config.sh`, and the `format_pi_stream` function. The main loop and other functions are added in later tasks. The script must already be source-safe (main loop guarded).

- [ ] **Step 8.1: Write `run-plan.sh` v1 (skeleton + format_pi_stream)**

```bash
#!/usr/bin/env bash
# Usage: run-plan.sh <plan-dir>
#
# Drives the four-phase work cycle for a backlog plan, using pi
# (@mariozechner/pi-coding-agent) as the LLM harness. Reads the
# current phase from <plan-dir>/phase.md, composes a prompt from the
# shared phases/<phase>.md file plus an optional
# <plan-dir>/prompt-<phase>.md, substitutes placeholders
# ({{DEV_ROOT}}, {{PROJECT}}, {{PLAN}}, {{RELATED_PLANS}}), and
# invokes pi.
#
# The work phase runs interactively (user converses and /exits when
# done). The reflect, compact, and triage phases run headless
# (pi --mode json -p ...). After triage exits, the script reads
# <plan-dir>/propagation.out.yaml (if present) and dispatches a fresh
# pi process per propagation target.
#
# Before launching pi for the work phase, the script deletes any stale
# latest-session.md and, if <plan-dir>/pre-work.sh exists and is
# executable, runs it from the project root. A non-zero hook exit
# aborts the whole cycle.
#
# The script exits cleanly if a phase does not advance phase.md. That
# is both the kill mechanism (work: user /exits without advancing
# phase.md) and error detection (headless phase crashes without
# completing). Ctrl-C in any phase also kills the cycle.

set -eu

# -----------------------------------------------------------------------------
# Self-location and configuration
# -----------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LLM_CONTEXT_PI_DIR="$SCRIPT_DIR"

# Defensive defaults — overridden by config.sh below if present.
HEADROOM=1500
PROVIDER="anthropic"
WORK_MODEL=""
REFLECT_MODEL=""
COMPACT_MODEL=""
TRIAGE_MODEL=""
WORK_THINKING=""
REFLECT_THINKING=""
COMPACT_THINKING=""
TRIAGE_THINKING=""

if [ -f "$LLM_CONTEXT_PI_DIR/config.sh" ]; then
    # shellcheck source=/dev/null
    . "$LLM_CONTEXT_PI_DIR/config.sh"
fi

# -----------------------------------------------------------------------------
# format_pi_stream — turn pi's --mode json firehose into a readable
# trace showing tool calls (with file path / pattern / command hints)
# and the final assistant text. Requires jq.
#
# Pi event shape is documented in:
#   @mariozechner/pi-agent-core/dist/types.d.ts:284-322
#   @mariozechner/pi-coding-agent/dist/core/agent-session.d.ts:40-65
# -----------------------------------------------------------------------------

format_pi_stream() {
    jq -j --unbuffered '
        def tool_summary:
          .toolName as $n |
          (.args // {}) as $a |
          "\n→ " + $n +
          (if $n == "read" or $n == "write" or $n == "edit" then
             (if $a.path then " " + $a.path
              elif $a.file_path then " " + $a.file_path
              else "" end)
           elif $n == "find" then
             (if $a.pattern then " " + $a.pattern else "" end)
             + (if $a.path then " (in " + $a.path + ")" else "" end)
           elif $n == "grep" then
             (if $a.pattern then " /" + $a.pattern + "/" else "" end)
             + (if $a.path then " in " + $a.path else "" end)
           elif $n == "ls" then
             (if $a.path then " " + $a.path else "" end)
           elif $n == "bash" then
             (if $a.command then
                " " + ($a.command | gsub("\n"; " ⏎ ")
                                  | if length > 120 then .[0:117] + "…" else . end)
              else "" end)
           else "" end)
          + "\n";

        if .type == "tool_execution_start" then
            tool_summary
        elif .type == "message_end" then
            # Extract the assistant text content from the final message.
            (.message.content // []
             | map(select(.type == "text") | .text)
             | join(""))
        elif .type == "tool_execution_end" and (.isError == true) then
            "\n[tool error: " + (.toolName // "?") + "]\n"
        else empty end
    '
}

# -----------------------------------------------------------------------------
# Main loop (guarded — only runs if this script is executed directly,
# not when sourced from test files).
# -----------------------------------------------------------------------------

if [ "${BASH_SOURCE[0]:-}" = "${0:-}" ]; then
    echo "run-plan.sh: main loop not yet implemented (task 8 skeleton)" >&2
    exit 1
fi
```

- [ ] **Step 8.2: Make it executable**

```bash
chmod +x /Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh
```

- [ ] **Step 8.3: Run the format_pi_stream test — should now PASS**

```bash
/Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh
```

Expected: all 9 assertions pass, exit code 0.

- [ ] **Step 8.4: If any test fails, fix `format_pi_stream` until all pass**

Most likely failure modes:
- jq filter doesn't extract text correctly from `message_end` — check the `.message.content` path shape.
- BSD vs GNU jq behavior differences — if `-j` misbehaves, try `-r` with explicit newlines.
- Fixture has a typo — fix the fixture if the parser is correct.

- [ ] **Step 8.5: Shellcheck**

```bash
shellcheck /Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh
```

Expected: no output or only SC2034 (unused variable) warnings for the defensive defaults (which are intentional). Fix any real warnings.

- [ ] **Step 8.6: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add run-plan.sh
git commit -m "Add run-plan.sh skeleton with format_pi_stream"
```

---

## Task 9: Propagation yaml fixture and parser test

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/test/fixtures/propagation-sample.yaml`
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/test/test-parse-propagation.sh`

- [ ] **Step 9.1: Write `propagation-sample.yaml`**

```yaml
propagations:
  - target: /Users/x/Development/SomeApp/LLM_STATE/core
    kind: child
    summary: |
      The collector now supports C function extraction, which changes
      how downstream apps should request symbol metadata. Affected
      consumers: SomeApp (the symbol-lookup frontend).
  - target: /Users/x/Development/Mnemosyne/LLM_STATE/harness
    kind: parent
    summary: |
      Identified a gap in the orchestrator's phase-kill semantics: when
      a pre-work hook fails, the kill propagates but the session log
      entry is not appended. Worth tracking upstream.
```

- [ ] **Step 9.2: Write the parser test**

```bash
#!/usr/bin/env bash
# Test for parse_propagation() in run-plan.sh.
# parse_propagation reads propagation.out.yaml on stdin and prints
# one tab-separated line per entry:
#   <kind>\t<target>\t<summary-single-line>
# The summary is compressed to a single line with \n sequences escaped
# to spaces.

set -eu

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"

# shellcheck source=../run-plan.sh
. "$ROOT/run-plan.sh"

pass=0
fail=0

out="$(parse_propagation < "$HERE/fixtures/propagation-sample.yaml")"
line_count="$(printf '%s\n' "$out" | grep -c .)"

if [ "$line_count" -eq 2 ]; then
    printf 'PASS line count (2)\n'
    pass=$((pass + 1))
else
    printf 'FAIL line count — expected 2, got %d\n' "$line_count"
    fail=$((fail + 1))
fi

line1="$(printf '%s\n' "$out" | sed -n 1p)"
line2="$(printf '%s\n' "$out" | sed -n 2p)"

case "$line1" in
    "child"$'\t'"/Users/x/Development/SomeApp/LLM_STATE/core"$'\t'*)
        printf 'PASS line1 kind+target\n'; pass=$((pass + 1)) ;;
    *)
        printf 'FAIL line1 kind+target: %s\n' "$line1"; fail=$((fail + 1)) ;;
esac

case "$line2" in
    "parent"$'\t'"/Users/x/Development/Mnemosyne/LLM_STATE/harness"$'\t'*)
        printf 'PASS line2 kind+target\n'; pass=$((pass + 1)) ;;
    *)
        printf 'FAIL line2 kind+target: %s\n' "$line2"; fail=$((fail + 1)) ;;
esac

case "$line1" in
    *"C function extraction"*)
        printf 'PASS line1 summary content\n'; pass=$((pass + 1)) ;;
    *)
        printf 'FAIL line1 summary content: %s\n' "$line1"; fail=$((fail + 1)) ;;
esac

case "$line2" in
    *"phase-kill semantics"*)
        printf 'PASS line2 summary content\n'; pass=$((pass + 1)) ;;
    *)
        printf 'FAIL line2 summary content: %s\n' "$line2"; fail=$((fail + 1)) ;;
esac

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
```

- [ ] **Step 9.3: Make executable and run — should FAIL (parse_propagation not defined)**

```bash
chmod +x /Users/antony/Development/LLM_CONTEXT_PI/test/test-parse-propagation.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-parse-propagation.sh || echo "failed as expected"
```

Expected: "parse_propagation: command not found" or similar, then "failed as expected".

- [ ] **Step 9.4: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add test/fixtures/propagation-sample.yaml test/test-parse-propagation.sh
git commit -m "Add failing test for parse_propagation"
```

---

## Task 10: Implement `parse_propagation()` in `run-plan.sh`

**Files:**
- Modify: `/Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh`

Hand-roll a narrow YAML parser for the specific `propagation.out.yaml` shape — no external YAML library required. The shape is known and fixed: a top-level `propagations:` list of mappings with `target`, `kind`, `summary` keys.

- [ ] **Step 10.1: Add `parse_propagation()` below `format_pi_stream()` in run-plan.sh**

```bash
# -----------------------------------------------------------------------------
# parse_propagation — read propagation.out.yaml on stdin, emit one
# tab-separated line per entry:
#   <kind>\t<target>\t<summary-joined-to-one-line>
#
# Format assumptions (fixed by phases/triage.md):
#   propagations:
#     - target: <absolute path>
#       kind: child|parent|sibling
#       summary: |
#         multi-line text
#         possibly several paragraphs
#     - target: ...
#
# The summary block scalar is recognised by trailing '|' after 'summary:',
# and is terminated by the next top-level-indented `- target:` line or EOF.
# Leading indentation (matching the first non-empty summary line) is
# stripped; internal line breaks become spaces.
# -----------------------------------------------------------------------------

parse_propagation() {
    awk '
        function flush() {
            if (have_entry) {
                gsub(/\t/, " ", summary)
                gsub(/[[:space:]]+$/, "", summary)
                printf "%s\t%s\t%s\n", kind, target, summary
            }
            target = ""; kind = ""; summary = ""
            in_summary = 0; summary_indent = -1
            have_entry = 0
        }
        BEGIN { have_entry = 0; in_summary = 0 }
        /^[[:space:]]*$/ {
            if (in_summary && summary != "") summary = summary " "
            next
        }
        /^propagations:[[:space:]]*$/ { next }
        /^[[:space:]]*-[[:space:]]+target:/ {
            flush()
            sub(/^[[:space:]]*-[[:space:]]+target:[[:space:]]*/, "")
            target = $0
            have_entry = 1
            next
        }
        /^[[:space:]]+kind:/ {
            sub(/^[[:space:]]+kind:[[:space:]]*/, "")
            kind = $0
            next
        }
        /^[[:space:]]+summary:[[:space:]]*\|[[:space:]]*$/ {
            in_summary = 1
            summary_indent = -1
            next
        }
        {
            if (in_summary) {
                line = $0
                if (summary_indent == -1) {
                    match(line, /^[[:space:]]*/)
                    summary_indent = RLENGTH
                }
                if (length(line) >= summary_indent) {
                    line = substr(line, summary_indent + 1)
                }
                if (summary == "") summary = line
                else summary = summary " " line
            }
        }
        END { flush() }
    '
}
```

- [ ] **Step 10.2: Run the propagation test**

```bash
/Users/antony/Development/LLM_CONTEXT_PI/test/test-parse-propagation.sh
```

Expected: all 5 assertions pass, exit code 0.

- [ ] **Step 10.3: Fix parser until all assertions pass**

Likely failure modes:
- `summary` field picks up trailing blank lines → handled by the `gsub(/[[:space:]]+$/, "", summary)` in `flush()`; if it still breaks, tighten the regex.
- Indentation mis-strip on the first line of summary → check the `summary_indent` capture.

- [ ] **Step 10.4: Re-run format test to ensure no regression**

```bash
/Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh
```

Expected: still passes.

- [ ] **Step 10.5: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add run-plan.sh
git commit -m "Implement parse_propagation yaml reader for triage output"
```

---

## Task 11: Port `list_plans_in`, `parse_related_projects`, `build_related_plans`

**Files:**
- Modify: `/Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh`

These three functions from `LLM_CONTEXT/run-plan.sh:155-245` port unchanged — no harness coupling. Copy them into the pi `run-plan.sh` below `parse_propagation()`.

- [ ] **Step 11.1: Copy the three functions from LLM_CONTEXT**

Read `/Users/antony/Development/LLM_CONTEXT/run-plan.sh` lines 155-245 and insert `list_plans_in`, `parse_related_projects`, and `build_related_plans` into the pi `run-plan.sh` below `parse_propagation()`. Copy verbatim — no changes.

- [ ] **Step 11.2: Shellcheck**

```bash
shellcheck /Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh
```

Expected: same clean/warning state as before task 11. Address any new issues.

- [ ] **Step 11.3: Re-run both existing tests**

```bash
/Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-parse-propagation.sh
```

Expected: both pass.

- [ ] **Step 11.4: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add run-plan.sh
git commit -m "Port related-plans helpers verbatim from LLM_CONTEXT"
```

---

## Task 12: Write compose_prompt test and port the function

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/test/test-compose-prompt.sh`
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/test/fixtures/tmp-phase/phase.md`
- Modify: `/Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh`

- [ ] **Step 12.1: Write the compose_prompt test**

```bash
#!/usr/bin/env bash
# Test for compose_prompt() in run-plan.sh.
#
# compose_prompt substitutes {{PROJECT}}, {{DEV_ROOT}}, {{PLAN}}, and
# {{RELATED_PLANS}} in both phases/<phase>.md and prompt-<phase>.md
# (if present) and prints the combined result.

set -eu

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"

# shellcheck source=../run-plan.sh
. "$ROOT/run-plan.sh"

TMP="$HERE/tmp"
rm -rf "$TMP"
mkdir -p "$TMP/plan"

# Fake project root with a git dir
mkdir -p "$TMP/project/.git"
# Fake plan dir under project
mv "$TMP/plan" "$TMP/project/LLM_STATE"
mkdir -p "$TMP/project/LLM_STATE/testplan"
echo work > "$TMP/project/LLM_STATE/testplan/phase.md"

# Override DIR / PROJECT / DEV_ROOT as the main loop would set them
DIR="$TMP/project/LLM_STATE/testplan"
PROJECT="$TMP/project"
DEV_ROOT="$TMP"

out="$(compose_prompt work)"

pass=0; fail=0

assert_contains() {
    if printf '%s' "$out" | grep -qF "$1"; then
        printf 'PASS %s\n' "$2"; pass=$((pass + 1))
    else
        printf 'FAIL %s — expected %s\n' "$2" "$1"; fail=$((fail + 1))
    fi
}

assert_not_contains() {
    if printf '%s' "$out" | grep -qF "$1"; then
        printf 'FAIL %s — should not contain %s\n' "$2" "$1"; fail=$((fail + 1))
    else
        printf 'PASS %s\n' "$2"; pass=$((pass + 1))
    fi
}

assert_contains "$PROJECT/README.md" "project path substituted"
assert_contains "$DIR/backlog.md" "plan path substituted"
assert_contains "$DEV_ROOT/LLM_CONTEXT_PI/fixed-memory" "dev_root substituted"
assert_not_contains "{{PROJECT}}" "no unsubstituted project placeholder"
assert_not_contains "{{DEV_ROOT}}" "no unsubstituted dev_root placeholder"
assert_not_contains "{{PLAN}}" "no unsubstituted plan placeholder"

rm -rf "$TMP"

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
```

- [ ] **Step 12.2: Make executable and run — should FAIL**

```bash
chmod +x /Users/antony/Development/LLM_CONTEXT_PI/test/test-compose-prompt.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-compose-prompt.sh || echo "failed as expected"
```

Expected: fails because `compose_prompt` is not yet defined.

- [ ] **Step 12.3: Commit the failing test**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add test/test-compose-prompt.sh
git commit -m "Add failing test for compose_prompt"
```

- [ ] **Step 12.4: Port `compose_prompt` from LLM_CONTEXT**

Read `LLM_CONTEXT/run-plan.sh:252-308` and insert `compose_prompt` into the pi `run-plan.sh` below `build_related_plans`. Port verbatim — no changes needed. The function works on phase files regardless of their content, so it does not care that triage.md is different.

- [ ] **Step 12.5: Run all three tests**

```bash
/Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-parse-propagation.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-compose-prompt.sh
```

Expected: all three pass.

- [ ] **Step 12.6: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add run-plan.sh
git commit -m "Port compose_prompt verbatim from LLM_CONTEXT"
```

---

## Task 13: Main loop — phase selection, pi invocation, post-phase bookkeeping

**Files:**
- Modify: `/Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh`

This is the biggest chunk of new code. Replace the placeholder main-loop-not-implemented block at the bottom of `run-plan.sh` with the real main loop.

- [ ] **Step 13.1: Replace the placeholder with the real main loop**

Delete the placeholder:

```bash
if [ "${BASH_SOURCE[0]:-}" = "${0:-}" ]; then
    echo "run-plan.sh: main loop not yet implemented (task 8 skeleton)" >&2
    exit 1
fi
```

Insert below all function definitions:

```bash
# -----------------------------------------------------------------------------
# Main loop (guarded — only runs if this script is executed directly,
# not when sourced from test files).
# -----------------------------------------------------------------------------

if [ "${BASH_SOURCE[0]:-}" = "${0:-}" ]; then
    PLAN_ARG=""

    for arg in "$@"; do
        case "$arg" in
            -*)
                echo "Unknown option: $arg" >&2
                echo "Usage: $0 <plan-dir>" >&2
                exit 1
                ;;
            *)
                if [ -n "$PLAN_ARG" ]; then
                    echo "Unexpected extra argument: $arg" >&2
                    exit 1
                fi
                PLAN_ARG="$arg"
                ;;
        esac
    done

    if [ -z "$PLAN_ARG" ]; then
        echo "Usage: $0 <plan-dir>" >&2
        exit 1
    fi

    if [ ! -d "$PLAN_ARG" ]; then
        echo "Error: $PLAN_ARG is not a directory" >&2
        exit 1
    fi

    DIR="$(cd "$PLAN_ARG" && pwd)"

    # Walk up from the plan dir to find the project root (.git)
    PROJECT="$DIR"
    while [ ! -d "$PROJECT/.git" ] && [ "$PROJECT" != "/" ]; do
        PROJECT="$(dirname "$PROJECT")"
    done
    if [ "$PROJECT" = "/" ]; then
        echo "Error: no git project root found above $DIR" >&2
        exit 1
    fi

    DEV_ROOT="$(dirname "$PROJECT")"
    PLAN_NAME="$(basename "$DIR")"

    SYSTEM_PROMPT_FILE="$LLM_CONTEXT_PI_DIR/system-prompt.md"
    if [ ! -f "$SYSTEM_PROMPT_FILE" ]; then
        echo "Error: $SYSTEM_PROMPT_FILE missing" >&2
        exit 1
    fi
    SYSTEM_PROMPT_CONTENT="$(cat "$SYSTEM_PROMPT_FILE")"

    # -------------------------------------------------------------------------
    # Auto-memory system: compute project memory directory
    # Mirrors Claude Code's ~/.claude/projects/<path>/memory/ convention.
    # -------------------------------------------------------------------------
    MEMORY_PROMPT_FILE="$LLM_CONTEXT_PI_DIR/memory-prompt.md"
    PROJECT_PATH_ENCODED="$(printf '%s' "$PROJECT" | tr '/' '-')"
    MEMORY_DIR="$HOME/.claude-pi/projects/$PROJECT_PATH_ENCODED/memory"
    MEMORY_INDEX="$MEMORY_DIR/MEMORY.md"
    mkdir -p "$MEMORY_DIR"

    # Read memory-prompt.md and substitute {{MEMORY_DIR}}
    MEMORY_PROMPT_CONTENT=""
    if [ -f "$MEMORY_PROMPT_FILE" ]; then
        MEMORY_PROMPT_CONTENT="$(sed "s|{{MEMORY_DIR}}|$MEMORY_DIR|g" "$MEMORY_PROMPT_FILE")"
    fi

    # Read MEMORY.md index if present
    MEMORY_INDEX_CONTENT=""
    if [ -f "$MEMORY_INDEX" ]; then
        MEMORY_INDEX_CONTENT="$(cat "$MEMORY_INDEX")"
    fi

    while true; do
        PHASE=$(cat "$DIR/phase.md" 2>/dev/null || echo work)
        PROMPT="$(compose_prompt "$PHASE")"
        printf '\n=== %s ===\n' "$PHASE"

        case "$PHASE" in
            work)    PHASE_MODEL="$WORK_MODEL";    PHASE_THINKING="$WORK_THINKING"    ;;
            reflect) PHASE_MODEL="$REFLECT_MODEL"; PHASE_THINKING="$REFLECT_THINKING" ;;
            compact) PHASE_MODEL="$COMPACT_MODEL"; PHASE_THINKING="$COMPACT_THINKING" ;;
            triage)  PHASE_MODEL="$TRIAGE_MODEL";  PHASE_THINKING="$TRIAGE_THINKING"  ;;
            *)
                echo "Error: unknown phase '$PHASE' in $DIR/phase.md" >&2
                exit 1
                ;;
        esac

        PI_ARGS=(--no-session --append-system-prompt "$SYSTEM_PROMPT_CONTENT")
        if [ -n "$PROVIDER" ]; then
            PI_ARGS+=(--provider "$PROVIDER")
        fi
        if [ -n "$PHASE_MODEL" ]; then
            PI_ARGS+=(--model "$PHASE_MODEL")
        fi
        if [ -n "$PHASE_THINKING" ]; then
            PI_ARGS+=(--thinking "$PHASE_THINKING")
        fi

        # Auto-memory injection (phase-dependent):
        # - Work phase: full read+write memory (instructions + index)
        # - Headless phases: read-only context (index only, no write instructions)
        if [ "$PHASE" = work ] && [ -n "$MEMORY_PROMPT_CONTENT" ]; then
            PI_ARGS+=(--append-system-prompt "$MEMORY_PROMPT_CONTENT")
        fi
        if [ -n "$MEMORY_INDEX_CONTENT" ]; then
            PI_ARGS+=(--append-system-prompt "## Current Memory Index
$MEMORY_INDEX_CONTENT")
        fi

        case "$PHASE" in
            work)
                rm -f "$DIR/latest-session.md"
                if [ -x "$DIR/pre-work.sh" ]; then
                    printf '\n=== pre-work hook ===\n'
                    if ! (cd "$PROJECT" && "$DIR/pre-work.sh"); then
                        echo "Error: $DIR/pre-work.sh failed — aborting cycle" >&2
                        exit 1
                    fi
                fi
                (cd "$PROJECT" && pi "${PI_ARGS[@]}" "$PROMPT")
                ;;
            reflect|compact|triage)
                (cd "$PROJECT" && pi "${PI_ARGS[@]}" --mode json -p "$PROMPT" \
                 | format_pi_stream)
                printf '\n'
                ;;
        esac

        NEW_PHASE=$(cat "$DIR/phase.md" 2>/dev/null || echo work)

        if [ "$PHASE" = "$NEW_PHASE" ]; then
            printf '\n=== %s did not advance phase.md — exiting ===\n' "$PHASE"
            exit 0
        fi

        # Session-log append (work only, guarded on advance).
        if [ "$PHASE" = work ] && [ -s "$DIR/latest-session.md" ]; then
            printf '\n' >> "$DIR/session-log.md"
            cat "$DIR/latest-session.md" >> "$DIR/session-log.md"
        fi

        # Compact-baseline update (compact only, guarded on advance).
        if [ "$PHASE" = compact ]; then
            wc -w < "$DIR/memory.md" 2>/dev/null | awk '{print $1}' > "$DIR/compact-baseline"
        fi

        # Reflect-to-compact relative trigger.
        if [ "$PHASE" = reflect ]; then
            BASELINE=$(cat "$DIR/compact-baseline" 2>/dev/null || echo 0)
            WORDS=$(wc -w < "$DIR/memory.md" 2>/dev/null | awk '{print $1}')
            WORDS=${WORDS:-0}
            if [ "$WORDS" -le $((BASELINE + HEADROOM)) ]; then
                echo triage > "$DIR/phase.md"
            fi
        fi

        # After triage: dispatch cross-plan propagations (if any).
        if [ "$PHASE" = triage ] && [ -f "$DIR/propagation.out.yaml" ]; then
            propagation_count=0
            while IFS=$'\t' read -r p_kind p_target p_summary; do
                if [ -z "${p_target:-}" ]; then
                    continue
                fi
                if [ ! -d "$p_target" ]; then
                    printf '\n=== propagation skip: %s does not exist ===\n' "$p_target" >&2
                    continue
                fi
                propagation_count=$((propagation_count + 1))
                printf '\n=== propagation → %s (%s) ===\n' "$p_target" "$p_kind"

                PROPAGATION_PROMPT="You are receiving a cross-plan propagation from the LLM_CONTEXT_PI system.

Source plan: $DIR
This plan: $p_target
Relationship: the source is your $p_kind

Learning from the source plan:
$p_summary

Read this plan's backlog.md and memory.md at $p_target, decide what (if anything) should be added to backlog.md or updated in memory.md as a result of the learning above, apply the changes using the edit and write tools, and return a one-line summary of what you did (or 'no changes needed' if you determined no update was warranted). Do not commit."

                (cd "$PROJECT" && pi --no-session \
                    --append-system-prompt "$SYSTEM_PROMPT_CONTENT" \
                    ${PROVIDER:+--provider "$PROVIDER"} \
                    ${TRIAGE_MODEL:+--model "$TRIAGE_MODEL"} \
                    --mode json -p "$PROPAGATION_PROMPT" \
                  | format_pi_stream)
                printf '\n'
            done < <(parse_propagation < "$DIR/propagation.out.yaml")

            # Clean up the propagation file — it is single-use per cycle.
            if [ "$propagation_count" -gt 0 ]; then
                rm -f "$DIR/propagation.out.yaml"
            fi
        fi
    done
fi
```

- [ ] **Step 13.2: Shellcheck**

```bash
shellcheck /Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh
```

Expected: clean or only SC2086 warnings on the `${VAR:+...}` conditional args (intentional — the word-splitting is wanted here). Address any unexpected warnings.

- [ ] **Step 13.3: Re-run all three tests — main loop must not break sourcing**

```bash
/Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-parse-propagation.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-compose-prompt.sh
```

Expected: all three pass. The main loop is guarded on `BASH_SOURCE == $0`, so sourcing from tests does not trigger it.

- [ ] **Step 13.4: Smoke-test arg parsing (no pi call)**

```bash
/Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh 2>&1 || true
/Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh /nonexistent 2>&1 || true
/Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh --unknown-flag 2>&1 || true
```

Expected outputs (in order):
```
Usage: /Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh <plan-dir>
Error: /nonexistent is not a directory
Unknown option: --unknown-flag
Usage: /Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh <plan-dir>
```

- [ ] **Step 13.5: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add run-plan.sh
git commit -m "Add main loop: phase dispatch, pi invocation, propagation fan-out"
```

---

## Task 14: Write `README.md`

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/README.md`

- [ ] **Step 14.1: Write the README**

Port the LLM_CONTEXT README verbatim, then apply these edits:

1. Retitle: `# LLM_CONTEXT_PI` and swap first paragraph to reference pi.
2. Replace every `LLM_CONTEXT/` path reference with `LLM_CONTEXT_PI/`.
3. Replace every `run-plan.sh ~/Development/{project}/...` example with the `LLM_CONTEXT_PI` path.
4. Delete the `--dangerous` flag documentation (removed per design decision).
5. Replace the "Configuration" section with text describing `PROVIDER`, `WORK_MODEL`, `REFLECT_MODEL`, `COMPACT_MODEL`, `TRIAGE_MODEL`, and `*_THINKING` variables, plus `HEADROOM`.
6. Add a new section **"Harness: pi"** immediately after the first paragraph with this content:

```markdown
## Harness: pi

LLM_CONTEXT_PI drives `pi` (the `@mariozechner/pi-coding-agent` CLI
from <https://github.com/badlogic/pi-mono>) as its LLM harness. Pi is
installed globally via:

```bash
npm install -g @mariozechner/pi-coding-agent
```

Provider API keys are supplied via environment variables
(`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`, etc.) — see
`pi --help` for the full list. The provider and per-phase model are
set in `config.sh`; the default is Anthropic with Opus for the work
phase and Sonnet for the editorial phases.

Every pi invocation is passed:

- `--no-session` — each phase runs in a fresh in-memory session so no
  stale context leaks across the four-phase cycle.
- `--append-system-prompt <contents of system-prompt.md>` — injects
  the explicit invariants (fresh-context mandate, tool etiquette,
  path-placeholder rule, verification discipline, negative-read
  discipline, destructive-op discipline, tone).
- `--provider`, `--model`, `--thinking` from `config.sh`.
- `--mode json -p` for the headless trio (reflect, compact, triage).
```

7. Replace the "Phase 3: TRIAGE" subsection in the "Phase Cycle"
   section with the new externalized-propagation design:

```markdown
### Phase 3: TRIAGE (headless)

Read the task backlog and distilled memory with fresh eyes. Adjust the
plan. Emit a propagation list for cross-plan fan-out.

The triage phase:

1. Reads `backlog.md` and `memory.md`.
2. Consumes `{{RELATED_PLANS}}` for cross-plan context.
3. Reviews each task: relevance, priority, splitting.
4. Adds tasks from learnings, removes obsolete ones, reprioritizes.
5. Scans for embedded blockers and promotes them to top-level tasks.
6. **Writes `{{PLAN}}/propagation.out.yaml`** containing one entry per
   related plan that warrants a propagation. The format is a top-level
   `propagations:` list of `{ target, kind, summary }` mappings.
7. Writes `work` to `phase.md`.
8. Stops.

After triage exits, `run-plan.sh` reads `propagation.out.yaml` (if
present) and dispatches a fresh `pi --mode json -p` process per
propagation target. Each propagation runs in its own pi process with
its own in-memory session and reads only the target plan's files —
there is no shared context between propagations or between a
propagation and the parent triage phase. The `propagation.out.yaml`
file is deleted after successful fan-out.

This replaces the in-session subagent dispatch model used by the
Claude Code version of LLM_CONTEXT. The externalized approach is
cleaner (parent context never touches child files), inspectable (the
yaml is a durable record of what got propagated), and harness-neutral
(the same pattern would work under any agent CLI).
```

8. In the "Files" list at the bottom, add `system-prompt.md`,
   `memory-prompt.md`, and remove references to `--dangerous`.

9. Add a new section **"Auto-memory system"** after the "Harness: pi"
   section documenting:
   - Memory directory: `~/.claude-pi/projects/<path-encoded>/memory/`
     where `<path-encoded>` is the absolute project path with `/`
     replaced by `-`.
   - Memory types: user, feedback, project, reference.
   - Memory is read+write during the work phase (interactive, where
     corrections and preferences emerge) and read-only during headless
     phases (index contents injected but no write instructions).
   - Migration from Claude Code: `cp -r ~/.claude/projects/*/memory/
     ~/.claude-pi/projects/*/memory/` (one-time, file format is
     identical).
   - Reference `memory-prompt.md` for the full instruction set.

- [ ] **Step 14.2: Spot-check that the README compiles in a markdown preview sense**

```bash
wc -l /Users/antony/Development/LLM_CONTEXT_PI/README.md
grep -c "^#" /Users/antony/Development/LLM_CONTEXT_PI/README.md
```

Expected: ~500 lines (similar size to LLM_CONTEXT's README), at least 15 headings.

- [ ] **Step 14.3: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add README.md
git commit -m "Add README.md adapted for pi harness"
```

---

## Task 15: Write `create-plan.md`

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/create-plan.md`

- [ ] **Step 15.1: Port create-plan.md from LLM_CONTEXT**

```bash
cp /Users/antony/Development/LLM_CONTEXT/create-plan.md \
   /Users/antony/Development/LLM_CONTEXT_PI/create-plan.md
```

- [ ] **Step 15.2: Apply edits**

Use Edit to make these substitutions in `LLM_CONTEXT_PI/create-plan.md`:
- `LLM_CONTEXT/run-plan.sh` → `LLM_CONTEXT_PI/run-plan.sh`
- `LLM_CONTEXT/README.md` → `LLM_CONTEXT_PI/README.md`
- `fixed-memory/coding-style*.md` reference stays the same (directory
  name is unchanged)
- Add a sentence to the "prompt-triage.md" bullet: "Also absent in
  most plans — LLM_CONTEXT_PI's triage phase emits a propagation
  yaml rather than dispatching subagents, so there is nothing
  plan-specific to override."

- [ ] **Step 15.3: Commit**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git add create-plan.md
git commit -m "Add create-plan.md adapted for pi"
```

---

## Task 16: End-to-end dry-run with a throwaway plan

**Files:**
- Create: `/Users/antony/Development/LLM_CONTEXT_PI/test/dryrun/` (gitignored)

This is the only test that exercises the full pipeline. It uses a throwaway git repo and does NOT call pi (no API key is available in the dev environment); instead it verifies that:
1. arg parsing, project-root walkup, and phase file reading all work;
2. compose_prompt produces fully-substituted output;
3. the "phase did not advance" early-exit fires correctly.

- [ ] **Step 16.1: Create a throwaway project with a plan**

```bash
TMP=/tmp/llm_context_pi_dryrun
rm -rf "$TMP"
mkdir -p "$TMP/fakeproj/LLM_STATE/smoke"
cd "$TMP/fakeproj" && git init -q
cd "$TMP/fakeproj/LLM_STATE/smoke"
cat > backlog.md <<'EOF'
# Backlog

### Smoke test task `[smoke]`
- **Status:** not_started
- **Dependencies:** none
- **Description:** placeholder for dry-run
- **Results:** _pending_
EOF
echo "# Memory" > memory.md
touch session-log.md
echo work > phase.md
```

- [ ] **Step 16.2: Patch run-plan.sh temporarily to dump the composed prompt instead of calling pi**

This is done via an environment-variable escape hatch that we
permanently add to run-plan.sh. Add near the top of the main-loop's
phase-dispatch `case` block, just before the `(cd "$PROJECT" && pi ...)`
invocation:

```bash
if [ -n "${LLM_CONTEXT_PI_DRYRUN:-}" ]; then
    printf '\n--- DRY RUN: would invoke pi with: ---\n'
    printf '  provider=%s model=%s thinking=%s\n' "$PROVIDER" "$PHASE_MODEL" "$PHASE_THINKING"
    printf '  args=%s\n' "${PI_ARGS[*]}"
    printf '  prompt length=%d\n' "${#PROMPT}"
    printf '  prompt head:\n%s\n' "$(printf '%s' "$PROMPT" | head -5)"
    # Advance phase manually so the main loop can continue in dry run.
    case "$PHASE" in
        work)    echo reflect > "$DIR/phase.md" ;;
        reflect) echo compact > "$DIR/phase.md" ;;
        compact) echo triage  > "$DIR/phase.md" ;;
        triage)  echo work    > "$DIR/phase.md"
                 # In a dry run, bail after triage to avoid infinite loop.
                 rm -f "$DIR/propagation.out.yaml"
                 : > "$DIR/_dryrun_triage_done"
                 ;;
    esac
    if [ -f "$DIR/_dryrun_triage_done" ]; then
        rm -f "$DIR/_dryrun_triage_done"
        exit 0
    fi
    continue
fi
```

This escape hatch is permanent — it is valuable for future debugging
and is cheap (one env-var check per phase).

- [ ] **Step 16.3: Run the dry run**

```bash
LLM_CONTEXT_PI_DRYRUN=1 \
  /Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh \
  /tmp/llm_context_pi_dryrun/fakeproj/LLM_STATE/smoke 2>&1 | tee /tmp/llm_context_pi_dryrun.log
```

Expected output contains, in order:
1. `=== work ===`
2. `--- DRY RUN: would invoke pi with: ---`
3. `provider=anthropic model=claude-opus-4-6 thinking=medium`
4. The beginning of the work.md prompt with paths substituted (no `{{...}}` tokens)
5. `=== reflect ===` then its own dry-run block
6. `=== compact ===` then its own dry-run block **OR** skipped via the relative trigger (expected skip: memory.md is empty, baseline=0, HEADROOM=1500, so 0 ≤ 1500 → triage is forced)
7. `=== triage ===` then its own dry-run block
8. Exit cleanly after triage.

- [ ] **Step 16.4: Inspect the log for unsubstituted placeholders**

```bash
grep -F "{{" /tmp/llm_context_pi_dryrun.log || echo "no placeholders leaked"
```

Expected: `no placeholders leaked`.

- [ ] **Step 16.5: Run all unit tests once more**

```bash
/Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-parse-propagation.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-compose-prompt.sh
```

Expected: all pass.

- [ ] **Step 16.6: Clean up dryrun state and commit the dryrun escape hatch**

```bash
rm -rf /tmp/llm_context_pi_dryrun /tmp/llm_context_pi_dryrun.log
cd /Users/antony/Development/LLM_CONTEXT_PI
git add run-plan.sh
git commit -m "Add LLM_CONTEXT_PI_DRYRUN escape hatch for offline verification"
```

---

## Task 17: Final verification pass

- [ ] **Step 17.1: Shellcheck everything**

```bash
shellcheck /Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh \
           /Users/antony/Development/LLM_CONTEXT_PI/config.sh \
           /Users/antony/Development/LLM_CONTEXT_PI/test/*.sh
```

Expected: no errors. Only acceptable warnings are SC2034 on intentional defensive defaults and SC2086 on intentional `${VAR:+...}` expansions.

- [ ] **Step 17.2: Directory listing sanity check**

```bash
find /Users/antony/Development/LLM_CONTEXT_PI -type f -not -path '*/.git/*' | sort
```

Expected files (no more, no less, plus the docs/superpowers/plans/ entry):
```
/Users/antony/Development/LLM_CONTEXT_PI/.gitignore
/Users/antony/Development/LLM_CONTEXT_PI/LICENSE
/Users/antony/Development/LLM_CONTEXT_PI/README.md
/Users/antony/Development/LLM_CONTEXT_PI/config.sh
/Users/antony/Development/LLM_CONTEXT_PI/create-plan.md
/Users/antony/Development/LLM_CONTEXT_PI/docs/superpowers/plans/2026-04-16-pi-adaptation.md
/Users/antony/Development/LLM_CONTEXT_PI/fixed-memory/coding-style-rust.md
/Users/antony/Development/LLM_CONTEXT_PI/fixed-memory/coding-style.md
/Users/antony/Development/LLM_CONTEXT_PI/fixed-memory/memory-style.md
/Users/antony/Development/LLM_CONTEXT_PI/phases/compact.md
/Users/antony/Development/LLM_CONTEXT_PI/phases/reflect.md
/Users/antony/Development/LLM_CONTEXT_PI/phases/triage.md
/Users/antony/Development/LLM_CONTEXT_PI/phases/work.md
/Users/antony/Development/LLM_CONTEXT_PI/run-plan.sh
/Users/antony/Development/LLM_CONTEXT_PI/system-prompt.md
/Users/antony/Development/LLM_CONTEXT_PI/test/fixtures/pi-stream-empty.jsonl
/Users/antony/Development/LLM_CONTEXT_PI/test/fixtures/pi-stream-sample.jsonl
/Users/antony/Development/LLM_CONTEXT_PI/test/fixtures/propagation-sample.yaml
/Users/antony/Development/LLM_CONTEXT_PI/test/test-compose-prompt.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-format-pi-stream.sh
/Users/antony/Development/LLM_CONTEXT_PI/test/test-parse-propagation.sh
```

- [ ] **Step 17.3: Verify no references to Claude Code specifics leaked in**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
grep -rniE "claude|anthropic\.com|--dangerously-skip-permissions|stream-json|include-partial-messages" \
    --include='*.md' --include='*.sh' \
    -- . | grep -v docs/superpowers/plans/ | grep -v README.md
```

Expected: no matches (the plan file and README are allowed — plan
references the research, README mentions Anthropic as a provider).

- [ ] **Step 17.4: Final commit and log**

```bash
cd /Users/antony/Development/LLM_CONTEXT_PI
git log --oneline
```

Expected: a clean sequence of ~13-15 commits, one per task, with
readable messages.

---

## Self-review

**Spec coverage:**
- ✅ Option A (externalized propagation): Task 4 (triage.md), Task 9-10 (yaml parser), Task 13 (dispatch loop)
- ✅ Fork from LLM_CONTEXT: Task 1 (verbatim copies), Tasks 3, 4, 5, 14, 15 (adaptations)
- ✅ Coexist cleanly / merge-back story: system-prompt.md artifact (Task 2), triage externalization (Task 4) — both are harness-neutral improvements
- ✅ Auto-memory system: Task 2B (memory-prompt.md), Task 13 (memory dir computation, conditional injection per phase)
- ✅ Same models: Task 5 config.sh uses claude-opus-4-6 / claude-sonnet-4-6
- ✅ Remove `--dangerous`: Task 13 main loop has no `--dangerous` / `-permissions` handling; Task 17 grep verifies
- ✅ Unit tests for the three pure functions: Tasks 7, 9, 12
- ✅ Dry-run end-to-end verification: Task 16

**Placeholder scan:**
- No "TBD", "TODO", "implement later" anywhere.
- Every code block is complete and runnable.
- Every function referenced in a test exists in a prior task (format_pi_stream → Task 8, parse_propagation → Task 10, compose_prompt → Task 12, list_plans_in/parse_related_projects/build_related_plans → Task 11).

**Type/name consistency:**
- `PI_ARGS` (Task 13) — consistent with how it's used.
- `PHASE_MODEL`/`PHASE_THINKING` — defined per-phase-case in Task 13, consumed immediately.
- `format_pi_stream` / `parse_propagation` / `compose_prompt` — each function name used identically across test, implementation, and main loop.
- `propagation.out.yaml` — same filename in triage.md (Task 4), parser (Task 10), dispatch loop (Task 13), README (Task 14), fixtures (Task 9).

No gaps identified.
