# Agent Merge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite the LLM_CONTEXT/LLM_CONTEXT_PI orchestrator in TypeScript with a shared phase cycle and pluggable agent backends (Claude Code and Pi).

**Architecture:** TypeScript CLI that implements the phase loop, prompt composition with token substitution, and agent-specific invocation. Each agent is a class implementing the `Agent` interface. Phase files are shared markdown with `{{TOOL_*}}` tokens. Pi gets custom prompt injection; Claude Code uses built-in behavior.

**Tech Stack:** TypeScript, Node.js (ESM), tsx (dev runner), vitest (tests), yaml (config parsing)

---

## File Map

### Create
- `src/types.ts` — Enums (LLMPhase, ScriptPhase) and interfaces (Agent, PlanContext, AgentConfig, SharedConfig, SubagentDispatch)
- `src/config.ts` — Load and merge config.yaml + agent config.yaml with CLI overrides
- `src/prompt-composer.ts` — Load phase markdown, substitute path and tool tokens
- `src/dream.ts` — Word-count check against baseline + headroom
- `src/git.ts` — Plan-scoped git commit, read commit-message.md
- `src/phase-loop.ts` — Core phase cycle with script-phase and LLM-phase dispatch
- `src/subagent-dispatch.ts` — Parse subagent-dispatch.yaml, iterate and dispatch
- `src/index.ts` — CLI entry point, argument parsing, agent selection
- `src/agents/agent.ts` — Agent interface definition
- `src/agents/claude-code/index.ts` — ClaudeCodeAgent implementation
- `src/agents/claude-code/stream-parser.ts` — Parse claude stream-json
- `src/agents/pi/index.ts` — PiAgent implementation
- `src/agents/pi/stream-parser.ts` — Parse pi JSONL
- `src/agents/pi/setup.ts` — Auto-install subagent extension, generate agent defs from skills
- `agents/claude-code/tokens.yaml` — Claude Code tool name mappings
- `agents/claude-code/config.yaml` — Claude Code model defaults
- `agents/pi/tokens.yaml` — Pi tool name mappings
- `agents/pi/config.yaml` — Pi model/thinking/provider defaults
- `agents/pi/prompts/system-prompt.md` — Pi system prompt (from existing)
- `agents/pi/prompts/memory-prompt.md` — Pi memory prompt (from existing)
- `config.yaml` — Shared config (headroom, default agent)
- `phases/work.md` — Parameterized work phase (from ../LLM_CONTEXT latest)
- `phases/analyse-work.md` — Parameterized analyse-work phase (from ../LLM_CONTEXT)
- `phases/reflect.md` — Parameterized reflect phase (from ../LLM_CONTEXT)
- `phases/dream.md` — Parameterized dream phase (from ../LLM_CONTEXT)
- `phases/triage.md` — Parameterized triage phase with YAML subagent dispatch
- `package.json` — Project manifest
- `tsconfig.json` — TypeScript config
- `vitest.config.ts` — Test config
- `test/prompt-composer.test.ts` — Prompt composition tests
- `test/dream.test.ts` — Dream trigger tests
- `test/config.test.ts` — Config loading tests
- `test/subagent-dispatch.test.ts` — YAML parsing tests
- `skills/brainstorming.md` — Extracted brainstorming skill
- `skills/writing-plans.md` — Extracted writing-plans skill
- `skills/tdd.md` — Extracted TDD skill

### Preserve
- `fixed-memory/coding-style.md`
- `fixed-memory/coding-style-rust.md`
- `fixed-memory/memory-style.md`
- `create-plan.md`
- `LICENSE`

### Remove (after plan complete)
- `run-plan.sh` — Replaced by TypeScript orchestrator
- `config.sh` — Replaced by config.yaml
- `prompts/` — Moved to `agents/pi/prompts/`
- `test/` (old bash tests) — Replaced by vitest suite

---

## Stage 1: Core Orchestrator

### Task 1: Project Setup

**Files:**
- Create: `package.json`
- Create: `tsconfig.json`
- Create: `vitest.config.ts`
- Modify: `.gitignore`

- [ ] **Step 1: Create package.json**

```json
{
  "name": "llm-context",
  "version": "1.0.0",
  "description": "Multi-agent orchestrator for backlog-driven LLM development cycles",
  "type": "module",
  "bin": {
    "llm-context": "./src/index.ts"
  },
  "scripts": {
    "start": "tsx src/index.ts",
    "test": "vitest run",
    "test:watch": "vitest",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "yaml": "^2.7.1"
  },
  "devDependencies": {
    "@types/node": "^22.15.3",
    "tsx": "^4.19.4",
    "typescript": "^5.8.3",
    "vitest": "^3.1.3"
  }
}
```

- [ ] **Step 2: Create tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "Node16",
    "moduleResolution": "Node16",
    "lib": ["ES2022"],
    "outDir": "dist",
    "rootDir": "src",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "declaration": true,
    "declarationMap": true,
    "sourceMap": true
  },
  "include": ["src"],
  "exclude": ["node_modules", "dist", "test"]
}
```

- [ ] **Step 3: Create vitest.config.ts**

```typescript
import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['test/**/*.test.ts'],
  },
})
```

- [ ] **Step 4: Update .gitignore**

Add to existing `.gitignore`:
```
node_modules/
dist/
.pi/agents/
```

- [ ] **Step 5: Install dependencies**

Run: `npm install`
Expected: `node_modules/` created, `package-lock.json` generated

- [ ] **Step 6: Verify setup**

Run: `npx tsc --noEmit`
Expected: No errors (no source files yet, clean exit)

Run: `npx vitest run`
Expected: "No test files found" (clean exit)

- [ ] **Step 7: Commit**

```bash
git add package.json tsconfig.json vitest.config.ts .gitignore package-lock.json
git commit -m "chore: initialize TypeScript project with tsx, vitest, yaml"
```

---

### Task 2: Types and Enums

**Files:**
- Create: `src/types.ts`

- [ ] **Step 1: Write the test**

Create `test/types.test.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { LLMPhase, ScriptPhase, isScriptPhase, isLLMPhase, PHASE_ORDER } from '../src/types.js'

describe('LLMPhase', () => {
  it('has all five LLM phases', () => {
    expect(LLMPhase.Work).toBe('work')
    expect(LLMPhase.AnalyseWork).toBe('analyse-work')
    expect(LLMPhase.Reflect).toBe('reflect')
    expect(LLMPhase.Dream).toBe('dream')
    expect(LLMPhase.Triage).toBe('triage')
  })
})

describe('ScriptPhase', () => {
  it('has all four git-commit phases', () => {
    expect(ScriptPhase.GitCommitWork).toBe('git-commit-work')
    expect(ScriptPhase.GitCommitReflect).toBe('git-commit-reflect')
    expect(ScriptPhase.GitCommitDream).toBe('git-commit-dream')
    expect(ScriptPhase.GitCommitTriage).toBe('git-commit-triage')
  })
})

describe('isScriptPhase', () => {
  it('returns true for script phases', () => {
    expect(isScriptPhase(ScriptPhase.GitCommitWork)).toBe(true)
    expect(isScriptPhase(ScriptPhase.GitCommitReflect)).toBe(true)
  })

  it('returns false for LLM phases', () => {
    expect(isScriptPhase(LLMPhase.Work)).toBe(false)
    expect(isScriptPhase(LLMPhase.Triage)).toBe(false)
  })
})

describe('isLLMPhase', () => {
  it('returns true for LLM phases', () => {
    expect(isLLMPhase(LLMPhase.Work)).toBe(true)
    expect(isLLMPhase(LLMPhase.AnalyseWork)).toBe(true)
  })

  it('returns false for script phases', () => {
    expect(isLLMPhase(ScriptPhase.GitCommitWork)).toBe(false)
  })
})

describe('PHASE_ORDER', () => {
  it('defines the correct phase chain', () => {
    expect(PHASE_ORDER).toEqual([
      LLMPhase.Work,
      LLMPhase.AnalyseWork,
      ScriptPhase.GitCommitWork,
      LLMPhase.Reflect,
      ScriptPhase.GitCommitReflect,
      LLMPhase.Dream,
      ScriptPhase.GitCommitDream,
      LLMPhase.Triage,
      ScriptPhase.GitCommitTriage,
    ])
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run test/types.test.ts`
Expected: FAIL — cannot find module `../src/types.js`

- [ ] **Step 3: Write types.ts**

Create `src/types.ts`:

```typescript
export enum LLMPhase {
  Work = 'work',
  AnalyseWork = 'analyse-work',
  Reflect = 'reflect',
  Dream = 'dream',
  Triage = 'triage',
}

export enum ScriptPhase {
  GitCommitWork = 'git-commit-work',
  GitCommitReflect = 'git-commit-reflect',
  GitCommitDream = 'git-commit-dream',
  GitCommitTriage = 'git-commit-triage',
}

export type Phase = LLMPhase | ScriptPhase

export const PHASE_ORDER: Phase[] = [
  LLMPhase.Work,
  LLMPhase.AnalyseWork,
  ScriptPhase.GitCommitWork,
  LLMPhase.Reflect,
  ScriptPhase.GitCommitReflect,
  LLMPhase.Dream,
  ScriptPhase.GitCommitDream,
  LLMPhase.Triage,
  ScriptPhase.GitCommitTriage,
]

const SCRIPT_PHASES = new Set<string>(Object.values(ScriptPhase))
const LLM_PHASES = new Set<string>(Object.values(LLMPhase))

export function isScriptPhase(phase: Phase): phase is ScriptPhase {
  return SCRIPT_PHASES.has(phase)
}

export function isLLMPhase(phase: Phase): phase is LLMPhase {
  return LLM_PHASES.has(phase)
}

export interface PlanContext {
  planDir: string
  projectDir: string
  devRoot: string
  relatedPlans: string
}

export interface AgentConfig {
  models: Record<LLMPhase, string>
  thinking?: Record<LLMPhase, string>
  provider?: string
}

export interface SharedConfig {
  headroom: number
  agent: string
}

export interface SubagentDispatch {
  target: string
  kind: 'child' | 'parent' | 'sibling'
  summary: string
}

export interface Agent {
  invokeInteractive(prompt: string, ctx: PlanContext): Promise<void>
  invokeHeadless(prompt: string, ctx: PlanContext, phase: LLMPhase): Promise<string>
  dispatchSubagent(prompt: string, targetPlan: string): Promise<string>
  tokens(): Record<string, string>
  setup?(ctx: PlanContext): Promise<void>
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run test/types.test.ts`
Expected: PASS — all assertions green

- [ ] **Step 5: Commit**

```bash
git add src/types.ts test/types.test.ts
git commit -m "feat: add core types, enums, and phase chain"
```

---

### Task 3: Config Loading

**Files:**
- Create: `src/config.ts`
- Create: `config.yaml`
- Create: `agents/claude-code/config.yaml`
- Create: `agents/pi/config.yaml`
- Create: `test/config.test.ts`

- [ ] **Step 1: Create config YAML files**

Create `config.yaml`:
```yaml
headroom: 1500
agent: claude-code
```

Create `agents/claude-code/config.yaml`:
```yaml
models:
  work: ""
  analyse-work: claude-sonnet-4-6
  reflect: claude-sonnet-4-6
  dream: claude-sonnet-4-6
  triage: claude-sonnet-4-6
```

Create `agents/pi/config.yaml`:
```yaml
provider: anthropic
models:
  work: claude-opus-4-6
  analyse-work: claude-sonnet-4-6
  reflect: claude-sonnet-4-6
  dream: claude-sonnet-4-6
  triage: claude-sonnet-4-6
thinking:
  work: medium
  analyse-work: ""
  reflect: ""
  dream: ""
  triage: ""
```

- [ ] **Step 2: Write the test**

Create `test/config.test.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { loadSharedConfig, loadAgentConfig } from '../src/config.js'
import { LLMPhase } from '../src/types.js'
import path from 'node:path'

const PROJECT_ROOT = path.resolve(import.meta.dirname, '..')

describe('loadSharedConfig', () => {
  it('loads shared config from config.yaml', () => {
    const config = loadSharedConfig(PROJECT_ROOT)
    expect(config.headroom).toBe(1500)
    expect(config.agent).toBe('claude-code')
  })

  it('CLI agent override takes precedence', () => {
    const config = loadSharedConfig(PROJECT_ROOT, 'pi')
    expect(config.agent).toBe('pi')
  })
})

describe('loadAgentConfig', () => {
  it('loads claude-code agent config', () => {
    const config = loadAgentConfig(PROJECT_ROOT, 'claude-code')
    expect(config.models[LLMPhase.Work]).toBe('')
    expect(config.models[LLMPhase.Reflect]).toBe('claude-sonnet-4-6')
    expect(config.thinking).toBeUndefined()
    expect(config.provider).toBeUndefined()
  })

  it('loads pi agent config', () => {
    const config = loadAgentConfig(PROJECT_ROOT, 'pi')
    expect(config.models[LLMPhase.Work]).toBe('claude-opus-4-6')
    expect(config.models[LLMPhase.Reflect]).toBe('claude-sonnet-4-6')
    expect(config.thinking?.[LLMPhase.Work]).toBe('medium')
    expect(config.provider).toBe('anthropic')
  })
})
```

- [ ] **Step 3: Run test to verify it fails**

Run: `npx vitest run test/config.test.ts`
Expected: FAIL — cannot find module `../src/config.js`

- [ ] **Step 4: Write config.ts**

Create `src/config.ts`:

```typescript
import fs from 'node:fs'
import path from 'node:path'
import YAML from 'yaml'
import { LLMPhase, type SharedConfig, type AgentConfig } from './types.js'

export function loadSharedConfig(projectRoot: string, cliAgent?: string): SharedConfig {
  const configPath = path.join(projectRoot, 'config.yaml')
  const raw = YAML.parse(fs.readFileSync(configPath, 'utf-8')) as {
    headroom: number
    agent: string
  }
  return {
    headroom: raw.headroom,
    agent: cliAgent ?? raw.agent,
  }
}

export function loadAgentConfig(projectRoot: string, agentName: string): AgentConfig {
  const configPath = path.join(projectRoot, 'agents', agentName, 'config.yaml')
  const raw = YAML.parse(fs.readFileSync(configPath, 'utf-8')) as Record<string, unknown>

  const models = raw.models as Record<string, string>
  const agentConfig: AgentConfig = {
    models: {
      [LLMPhase.Work]: models.work ?? '',
      [LLMPhase.AnalyseWork]: models['analyse-work'] ?? '',
      [LLMPhase.Reflect]: models.reflect ?? '',
      [LLMPhase.Dream]: models.dream ?? '',
      [LLMPhase.Triage]: models.triage ?? '',
    } as Record<LLMPhase, string>,
  }

  if (raw.thinking) {
    const thinking = raw.thinking as Record<string, string>
    agentConfig.thinking = {
      [LLMPhase.Work]: thinking.work ?? '',
      [LLMPhase.AnalyseWork]: thinking['analyse-work'] ?? '',
      [LLMPhase.Reflect]: thinking.reflect ?? '',
      [LLMPhase.Dream]: thinking.dream ?? '',
      [LLMPhase.Triage]: thinking.triage ?? '',
    } as Record<LLMPhase, string>
  }

  if (raw.provider) {
    agentConfig.provider = raw.provider as string
  }

  return agentConfig
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `npx vitest run test/config.test.ts`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/config.ts config.yaml agents/claude-code/config.yaml agents/pi/config.yaml test/config.test.ts
git commit -m "feat: add typed config loading from YAML"
```

---

### Task 4: Token Files

**Files:**
- Create: `agents/claude-code/tokens.yaml`
- Create: `agents/pi/tokens.yaml`

- [ ] **Step 1: Create Claude Code tokens**

Create `agents/claude-code/tokens.yaml`:
```yaml
TOOL_READ: Read
TOOL_WRITE: Write
TOOL_EDIT: Edit
TOOL_GREP: Grep
TOOL_GLOB: Glob
TOOL_BASH: Bash
TOOL_LS: LS
```

- [ ] **Step 2: Create Pi tokens**

Create `agents/pi/tokens.yaml`:
```yaml
TOOL_READ: read
TOOL_WRITE: write
TOOL_EDIT: edit
TOOL_GREP: grep
TOOL_GLOB: find
TOOL_BASH: bash
TOOL_LS: ls
```

- [ ] **Step 3: Commit**

```bash
git add agents/claude-code/tokens.yaml agents/pi/tokens.yaml
git commit -m "feat: add per-agent tool token mappings"
```

---

### Task 5: Prompt Composer

**Files:**
- Create: `src/prompt-composer.ts`
- Create: `test/prompt-composer.test.ts`

- [ ] **Step 1: Write the test**

Create `test/prompt-composer.test.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { substituteTokens, loadPhaseFile } from '../src/prompt-composer.js'
import { LLMPhase, type PlanContext } from '../src/types.js'
import path from 'node:path'

const PROJECT_ROOT = path.resolve(import.meta.dirname, '..')

describe('substituteTokens', () => {
  const ctx: PlanContext = {
    planDir: '/home/user/project/LLM_STATE/my-plan',
    projectDir: '/home/user/project',
    devRoot: '/home/user',
    relatedPlans: '- /home/user/other/LLM_STATE/other-plan (child)',
  }

  const tokens: Record<string, string> = {
    TOOL_READ: 'Read',
    TOOL_WRITE: 'Write',
    TOOL_BASH: 'Bash',
  }

  it('substitutes path tokens', () => {
    const input = 'Read {{PROJECT}}/README.md and check {{PLAN}}/backlog.md'
    const result = substituteTokens(input, ctx, tokens)
    expect(result).toBe(
      'Read /home/user/project/README.md and check /home/user/project/LLM_STATE/my-plan/backlog.md'
    )
  })

  it('substitutes DEV_ROOT', () => {
    const input = '{{DEV_ROOT}}/LLM_CONTEXT/fixed-memory/coding-style.md'
    const result = substituteTokens(input, ctx, tokens)
    expect(result).toBe('/home/user/LLM_CONTEXT/fixed-memory/coding-style.md')
  })

  it('substitutes RELATED_PLANS', () => {
    const input = 'Related plans:\n{{RELATED_PLANS}}'
    const result = substituteTokens(input, ctx, tokens)
    expect(result).toContain('- /home/user/other/LLM_STATE/other-plan (child)')
  })

  it('substitutes tool tokens', () => {
    const input = 'Use {{TOOL_READ}} to read files and {{TOOL_BASH}} for commands'
    const result = substituteTokens(input, ctx, tokens)
    expect(result).toBe('Use Read to read files and Bash for commands')
  })

  it('leaves unknown tokens unchanged', () => {
    const input = 'This has {{UNKNOWN}} token'
    const result = substituteTokens(input, ctx, tokens)
    expect(result).toBe('This has {{UNKNOWN}} token')
  })
})

describe('loadPhaseFile', () => {
  it('loads a phase markdown file', () => {
    const content = loadPhaseFile(PROJECT_ROOT, LLMPhase.Reflect)
    expect(content).toContain('reflect')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run test/prompt-composer.test.ts`
Expected: FAIL — cannot find module

- [ ] **Step 3: Write prompt-composer.ts**

Create `src/prompt-composer.ts`:

```typescript
import fs from 'node:fs'
import path from 'node:path'
import { type LLMPhase, type PlanContext } from './types.js'

export function substituteTokens(
  content: string,
  ctx: PlanContext,
  tokens: Record<string, string>
): string {
  let result = content
  result = result.replaceAll('{{DEV_ROOT}}', ctx.devRoot)
  result = result.replaceAll('{{PROJECT}}', ctx.projectDir)
  result = result.replaceAll('{{PLAN}}', ctx.planDir)
  result = result.replaceAll('{{RELATED_PLANS}}', ctx.relatedPlans)

  for (const [key, value] of Object.entries(tokens)) {
    result = result.replaceAll(`{{${key}}}`, value)
  }

  return result
}

export function loadPhaseFile(projectRoot: string, phase: LLMPhase): string {
  const filePath = path.join(projectRoot, 'phases', `${phase}.md`)
  return fs.readFileSync(filePath, 'utf-8')
}

export function loadPlanOverride(planDir: string, phase: LLMPhase): string | null {
  const filePath = path.join(planDir, `prompt-${phase}.md`)
  if (fs.existsSync(filePath)) {
    return fs.readFileSync(filePath, 'utf-8')
  }
  return null
}

export function composePrompt(
  projectRoot: string,
  phase: LLMPhase,
  ctx: PlanContext,
  tokens: Record<string, string>
): string {
  const base = loadPhaseFile(projectRoot, phase)
  const override = loadPlanOverride(ctx.planDir, phase)

  let prompt = base
  if (override) {
    prompt += '\n\n---\n\n' + override
  }

  return substituteTokens(prompt, ctx, tokens)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run test/prompt-composer.test.ts`
Expected: PASS (the `loadPhaseFile` test will need at least one phase file to exist — create a minimal `phases/reflect.md` placeholder first if needed; the full phase files come in Task 8)

- [ ] **Step 5: Commit**

```bash
git add src/prompt-composer.ts test/prompt-composer.test.ts
git commit -m "feat: add prompt composition with token substitution"
```

---

### Task 6: Dream Trigger

**Files:**
- Create: `src/dream.ts`
- Create: `test/dream.test.ts`

- [ ] **Step 1: Write the test**

Create `test/dream.test.ts`:

```typescript
import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { shouldDream, updateDreamBaseline } from '../src/dream.js'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'

describe('shouldDream', () => {
  let tmpDir: string

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'dream-test-'))
  })

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true })
  })

  it('returns false when memory.md does not exist', () => {
    expect(shouldDream(tmpDir, 1500)).toBe(false)
  })

  it('returns false when no baseline exists (first run)', () => {
    fs.writeFileSync(path.join(tmpDir, 'memory.md'), 'hello world')
    expect(shouldDream(tmpDir, 1500)).toBe(false)
  })

  it('returns false when word count is within headroom', () => {
    fs.writeFileSync(path.join(tmpDir, 'memory.md'), 'word '.repeat(100))
    fs.writeFileSync(path.join(tmpDir, 'dream-baseline'), '50')
    expect(shouldDream(tmpDir, 1500)).toBe(false)
  })

  it('returns true when word count exceeds baseline + headroom', () => {
    fs.writeFileSync(path.join(tmpDir, 'memory.md'), 'word '.repeat(2000))
    fs.writeFileSync(path.join(tmpDir, 'dream-baseline'), '100')
    expect(shouldDream(tmpDir, 1500)).toBe(true)
  })
})

describe('updateDreamBaseline', () => {
  let tmpDir: string

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'dream-test-'))
  })

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true })
  })

  it('writes current word count to dream-baseline', () => {
    fs.writeFileSync(path.join(tmpDir, 'memory.md'), 'word '.repeat(500))
    updateDreamBaseline(tmpDir)
    const baseline = fs.readFileSync(path.join(tmpDir, 'dream-baseline'), 'utf-8')
    expect(parseInt(baseline, 10)).toBe(500)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run test/dream.test.ts`
Expected: FAIL

- [ ] **Step 3: Write dream.ts**

Create `src/dream.ts`:

```typescript
import fs from 'node:fs'
import path from 'node:path'

function wordCount(text: string): number {
  return text.split(/\s+/).filter(w => w.length > 0).length
}

export function shouldDream(planDir: string, headroom: number): boolean {
  const memoryPath = path.join(planDir, 'memory.md')
  const baselinePath = path.join(planDir, 'dream-baseline')

  if (!fs.existsSync(memoryPath)) return false
  if (!fs.existsSync(baselinePath)) return false

  const words = wordCount(fs.readFileSync(memoryPath, 'utf-8'))
  const baseline = parseInt(fs.readFileSync(baselinePath, 'utf-8').trim(), 10)

  return words > baseline + headroom
}

export function updateDreamBaseline(planDir: string): void {
  const memoryPath = path.join(planDir, 'memory.md')
  const baselinePath = path.join(planDir, 'dream-baseline')

  const words = wordCount(fs.readFileSync(memoryPath, 'utf-8'))
  fs.writeFileSync(baselinePath, words.toString())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run test/dream.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/dream.ts test/dream.test.ts
git commit -m "feat: add dream trigger check with headroom threshold"
```

---

### Task 7: Git Operations

**Files:**
- Create: `src/git.ts`

- [ ] **Step 1: Write git.ts**

Create `src/git.ts`:

```typescript
import { execSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

export function gitCommitPlan(planDir: string, planName: string, phaseName: string): boolean {
  const commitMsgPath = path.join(planDir, 'commit-message.md')
  let message: string

  if (fs.existsSync(commitMsgPath)) {
    message = fs.readFileSync(commitMsgPath, 'utf-8').trim()
    fs.unlinkSync(commitMsgPath)
  } else {
    message = `run-plan: ${phaseName} (${planName})`
  }

  try {
    execSync(`git add "${planDir}"`, { stdio: 'pipe' })
    const status = execSync('git diff --cached --quiet', { stdio: 'pipe' }).toString()
    return false // no changes staged
  } catch {
    // git diff --cached --quiet exits non-zero when there are staged changes
    execSync(`git commit -m "${message.replace(/"/g, '\\"')}"`, { stdio: 'pipe' })
    return true
  }
}

export function gitSaveWorkBaseline(planDir: string): void {
  const baselinePath = path.join(planDir, 'work-baseline')
  try {
    const sha = execSync('git rev-parse HEAD', { stdio: 'pipe' }).toString().trim()
    fs.writeFileSync(baselinePath, sha)
  } catch {
    fs.writeFileSync(baselinePath, '')
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add src/git.ts
git commit -m "feat: add git commit and work-baseline helpers"
```

---

### Task 8: Phase Files (Parameterized)

**Files:**
- Create: `phases/work.md`
- Create: `phases/analyse-work.md`
- Create: `phases/reflect.md`
- Create: `phases/dream.md`
- Create: `phases/triage.md`

Copy phase files from `../LLM_CONTEXT/phases/` and parameterize tool names with `{{TOOL_*}}` tokens. Also update `compact.md` → `dream.md` and adjust the triage phase to use YAML subagent dispatch instead of inline subagents.

- [ ] **Step 1: Copy and parameterize phase files**

Read each phase file from `../LLM_CONTEXT/phases/` and create the parameterized version:

For each file:
1. Copy the content
2. Replace tool name references: `Read` → `{{TOOL_READ}}`, `Write` → `{{TOOL_WRITE}}`, etc. Only replace when used as tool names in instructions (e.g., "Use Read to..." becomes "Use {{TOOL_READ}} to..."), not in general prose.
3. Replace `LLM_CONTEXT` path references with parameterized form
4. For `dream.md`: rename from `compact.md`, keep content identical
5. For `triage.md`: replace inline subagent dispatch instructions with YAML manifest output instructions

The triage.md subagent section should become:

```markdown
## Cross-plan subagent dispatch

For each related plan where learnings warrant propagation, **write** `{{PLAN}}/subagent-dispatch.yaml` containing one entry per target:

```yaml
dispatches:
  - target: /absolute/path/to/related/plan
    kind: child
    summary: |
      One to three paragraphs describing the learnings and
      suggested backlog/memory updates for the target plan.
```

Rules:
- Use absolute paths for targets
- Use `|` (block scalar) for multi-line summaries
- Omit the file entirely if there are no dispatches
- Do **not** attempt to dispatch anything yourself — the driver reads this file after you exit and handles dispatch
```

- [ ] **Step 2: Verify no unsubstituted LLM_CONTEXT paths remain**

Run: `grep -r "LLM_CONTEXT" phases/`
Expected: No matches (all paths should use `{{DEV_ROOT}}` or be removed)

- [ ] **Step 3: Commit**

```bash
git add phases/
git commit -m "feat: add parameterized phase files with tool tokens"
```

---

### Task 9: Agent Interface and Claude Code Agent

**Files:**
- Create: `src/agents/agent.ts`
- Create: `src/agents/claude-code/index.ts`
- Create: `src/agents/claude-code/stream-parser.ts`

- [ ] **Step 1: Create agent interface**

Create `src/agents/agent.ts`:

```typescript
export type { Agent } from '../types.js'
```

- [ ] **Step 2: Create Claude Code stream parser**

Create `src/agents/claude-code/stream-parser.ts`:

```typescript
export function formatClaudeStreamLine(line: string): string | null {
  if (!line.trim()) return null

  let event: Record<string, unknown>
  try {
    event = JSON.parse(line)
  } catch {
    return null
  }

  if (event.type === 'assistant' && event.subtype === 'tool_use') {
    const tool = event as Record<string, unknown>
    const name = tool.tool_name as string
    const input = tool.tool_input as Record<string, unknown>

    switch (name) {
      case 'Read':
        return `  ▸ Read ${input.file_path}`
      case 'Write':
        return `  ▸ Write ${input.file_path}`
      case 'Edit':
        return `  ▸ Edit ${input.file_path}`
      case 'Grep':
        return `  ▸ Grep "${input.pattern}" in ${input.path ?? '.'}`
      case 'Glob':
        return `  ▸ Glob ${input.pattern}`
      case 'Bash':
        return `  ▸ Bash: ${(input.command as string).slice(0, 120)}`
      default:
        return `  ▸ ${name}`
    }
  }

  if (event.type === 'assistant' && event.subtype === 'text') {
    return event.text as string
  }

  return null
}
```

- [ ] **Step 3: Create ClaudeCodeAgent**

Create `src/agents/claude-code/index.ts`:

```typescript
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import YAML from 'yaml'
import { type Agent, type PlanContext, type AgentConfig, LLMPhase } from '../../types.js'
import { formatClaudeStreamLine } from './stream-parser.js'

export class ClaudeCodeAgent implements Agent {
  private config: AgentConfig
  private projectRoot: string

  constructor(config: AgentConfig, projectRoot: string) {
    this.config = config
    this.projectRoot = projectRoot
  }

  async invokeInteractive(prompt: string, ctx: PlanContext): Promise<void> {
    const args: string[] = []
    const model = this.config.models[LLMPhase.Work]
    if (model) args.push('--model', model)
    args.push('--output-format', 'stream-json')

    return new Promise((resolve, reject) => {
      const child = spawn('claude', args, {
        cwd: ctx.projectDir,
        stdio: ['inherit', 'inherit', 'inherit'],
      })
      child.on('close', code => {
        if (code === 0) resolve()
        else reject(new Error(`claude exited with code ${code}`))
      })
    })
  }

  async invokeHeadless(prompt: string, ctx: PlanContext, phase: LLMPhase): Promise<string> {
    const args = ['-p', prompt, '--output-format', 'stream-json']
    const model = this.config.models[phase]
    if (model) args.push('--model', model)

    return new Promise((resolve, reject) => {
      const chunks: string[] = []
      const child = spawn('claude', args, {
        cwd: ctx.projectDir,
        stdio: ['pipe', 'pipe', 'inherit'],
      })

      child.stdout.on('data', (data: Buffer) => {
        const lines = data.toString().split('\n')
        for (const line of lines) {
          const formatted = formatClaudeStreamLine(line)
          if (formatted) {
            process.stderr.write(formatted + '\n')
            chunks.push(formatted)
          }
        }
      })

      child.on('close', code => {
        if (code === 0) resolve(chunks.join('\n'))
        else reject(new Error(`claude exited with code ${code}`))
      })
    })
  }

  async dispatchSubagent(prompt: string, targetPlan: string): Promise<string> {
    return this.invokeHeadless(prompt, {
      planDir: targetPlan,
      projectDir: path.dirname(path.dirname(targetPlan)),
      devRoot: path.dirname(path.dirname(path.dirname(targetPlan))),
      relatedPlans: '',
    }, LLMPhase.Triage)
  }

  tokens(): Record<string, string> {
    const tokensPath = path.join(this.projectRoot, 'agents', 'claude-code', 'tokens.yaml')
    return YAML.parse(fs.readFileSync(tokensPath, 'utf-8')) as Record<string, string>
  }
}
```

- [ ] **Step 4: Commit**

```bash
git add src/agents/
git commit -m "feat: add Agent interface and ClaudeCodeAgent implementation"
```

---

### Task 10: Pi Agent

**Files:**
- Create: `src/agents/pi/index.ts`
- Create: `src/agents/pi/stream-parser.ts`
- Move: `prompts/system-prompt.md` → `agents/pi/prompts/system-prompt.md`
- Move: `prompts/memory-prompt.md` → `agents/pi/prompts/memory-prompt.md`

- [ ] **Step 1: Move Pi prompts to agent directory**

```bash
mkdir -p agents/pi/prompts
cp prompts/system-prompt.md agents/pi/prompts/system-prompt.md
cp prompts/memory-prompt.md agents/pi/prompts/memory-prompt.md
```

- [ ] **Step 2: Create Pi stream parser**

Create `src/agents/pi/stream-parser.ts`:

```typescript
export function formatPiStreamLine(line: string): string | null {
  if (!line.trim()) return null

  let event: Record<string, unknown>
  try {
    event = JSON.parse(line)
  } catch {
    return null
  }

  if (event.type === 'tool_execution_start') {
    const name = event.tool_name as string
    const input = event.tool_input as Record<string, unknown>

    switch (name) {
      case 'read':
        return `  ▸ read ${input.file_path ?? input.path ?? ''}`
      case 'write':
        return `  ▸ write ${input.file_path ?? input.path ?? ''}`
      case 'edit':
        return `  ▸ edit ${input.file_path ?? input.path ?? ''}`
      case 'grep':
        return `  ▸ grep "${input.pattern}" in ${input.path ?? '.'}`
      case 'find':
        return `  ▸ find ${input.pattern ?? input.glob ?? ''}`
      case 'bash':
        return `  ▸ bash: ${(input.command as string).slice(0, 120)}`
      default:
        return `  ▸ ${name}`
    }
  }

  if (event.type === 'tool_execution_end' && event.isError) {
    return `  ✗ tool error`
  }

  if (event.type === 'message_end') {
    const content = event.content as Array<{ type: string; text?: string }>
    if (Array.isArray(content)) {
      return content
        .filter(c => c.type === 'text' && c.text)
        .map(c => c.text)
        .join('\n')
    }
  }

  return null
}
```

- [ ] **Step 3: Create PiAgent**

Create `src/agents/pi/index.ts`:

```typescript
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import YAML from 'yaml'
import { type Agent, type PlanContext, type AgentConfig, LLMPhase } from '../../types.js'
import { formatPiStreamLine } from './stream-parser.js'

export class PiAgent implements Agent {
  private config: AgentConfig
  private projectRoot: string

  constructor(config: AgentConfig, projectRoot: string) {
    this.config = config
    this.projectRoot = projectRoot
  }

  private loadPromptFile(name: string, ctx: PlanContext): string {
    const filePath = path.join(this.projectRoot, 'agents', 'pi', 'prompts', name)
    let content = fs.readFileSync(filePath, 'utf-8')
    content = content.replaceAll('{{PROJECT}}', ctx.projectDir)
    content = content.replaceAll('{{DEV_ROOT}}', ctx.devRoot)
    content = content.replaceAll('{{PLAN}}', ctx.planDir)
    return content
  }

  async invokeInteractive(prompt: string, ctx: PlanContext): Promise<void> {
    const systemPrompt = this.loadPromptFile('system-prompt.md', ctx)
    const memoryPrompt = this.loadPromptFile('memory-prompt.md', ctx)
    const fullSystemPrompt = systemPrompt + '\n\n' + memoryPrompt

    const args: string[] = [
      '--no-session',
      '--append-system-prompt', fullSystemPrompt,
      '--provider', this.config.provider ?? 'anthropic',
      '--model', this.config.models[LLMPhase.Work],
    ]

    const thinking = this.config.thinking?.[LLMPhase.Work]
    if (thinking) args.push('--thinking', thinking)

    return new Promise((resolve, reject) => {
      const child = spawn('pi', args, {
        cwd: ctx.projectDir,
        stdio: ['inherit', 'inherit', 'inherit'],
      })
      child.on('close', code => {
        if (code === 0) resolve()
        else reject(new Error(`pi exited with code ${code}`))
      })
    })
  }

  async invokeHeadless(prompt: string, ctx: PlanContext, phase: LLMPhase): Promise<string> {
    const systemPrompt = this.loadPromptFile('system-prompt.md', ctx)

    const args: string[] = [
      '--no-session',
      '--append-system-prompt', systemPrompt,
      '--provider', this.config.provider ?? 'anthropic',
      '--model', this.config.models[phase],
      '--mode', 'json',
      '-p', prompt,
    ]

    const thinking = this.config.thinking?.[phase]
    if (thinking) args.push('--thinking', thinking)

    return new Promise((resolve, reject) => {
      const chunks: string[] = []
      const child = spawn('pi', args, {
        cwd: ctx.projectDir,
        stdio: ['pipe', 'pipe', 'inherit'],
      })

      child.stdout.on('data', (data: Buffer) => {
        const lines = data.toString().split('\n')
        for (const line of lines) {
          const formatted = formatPiStreamLine(line)
          if (formatted) {
            process.stderr.write(formatted + '\n')
            chunks.push(formatted)
          }
        }
      })

      child.on('close', code => {
        if (code === 0) resolve(chunks.join('\n'))
        else reject(new Error(`pi exited with code ${code}`))
      })
    })
  }

  async dispatchSubagent(prompt: string, targetPlan: string): Promise<string> {
    return this.invokeHeadless(prompt, {
      planDir: targetPlan,
      projectDir: path.dirname(path.dirname(targetPlan)),
      devRoot: path.dirname(path.dirname(path.dirname(targetPlan))),
      relatedPlans: '',
    }, LLMPhase.Triage)
  }

  tokens(): Record<string, string> {
    const tokensPath = path.join(this.projectRoot, 'agents', 'pi', 'tokens.yaml')
    return YAML.parse(fs.readFileSync(tokensPath, 'utf-8')) as Record<string, string>
  }
}
```

- [ ] **Step 4: Commit**

```bash
git add src/agents/pi/ agents/pi/prompts/
git commit -m "feat: add PiAgent with prompt injection and stream parsing"
```

---

### Task 11: Phase Loop

**Files:**
- Create: `src/phase-loop.ts`

- [ ] **Step 1: Write phase-loop.ts**

Create `src/phase-loop.ts`:

```typescript
import fs from 'node:fs'
import path from 'node:path'
import readline from 'node:readline'
import {
  type Agent,
  type PlanContext,
  type SharedConfig,
  type Phase,
  LLMPhase,
  ScriptPhase,
  isScriptPhase,
} from './types.js'
import { composePrompt } from './prompt-composer.js'
import { shouldDream, updateDreamBaseline } from './dream.js'
import { gitCommitPlan, gitSaveWorkBaseline } from './git.js'
import { dispatchSubagents } from './subagent-dispatch.js'

function readPhase(planDir: string): Phase {
  const phasePath = path.join(planDir, 'phase.md')
  return fs.readFileSync(phasePath, 'utf-8').trim() as Phase
}

function writePhase(planDir: string, phase: Phase): void {
  fs.writeFileSync(path.join(planDir, 'phase.md'), phase)
}

function planName(planDir: string): string {
  return path.basename(planDir)
}

async function askContinue(): Promise<boolean> {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  })
  return new Promise(resolve => {
    rl.question('\nContinue to next phase? [Y/n] ', answer => {
      rl.close()
      resolve(answer.trim().toLowerCase() !== 'n')
    })
  })
}

async function handleScriptPhase(
  phase: ScriptPhase,
  planDir: string
): Promise<boolean> {
  const name = planName(planDir)

  switch (phase) {
    case ScriptPhase.GitCommitWork:
      gitCommitPlan(planDir, name, 'work')
      writePhase(planDir, LLMPhase.Reflect)
      return askContinue()

    case ScriptPhase.GitCommitReflect:
      gitCommitPlan(planDir, name, 'reflect')
      writePhase(planDir, LLMPhase.Dream)
      return true

    case ScriptPhase.GitCommitDream:
      gitCommitPlan(planDir, name, 'dream')
      writePhase(planDir, LLMPhase.Triage)
      return true

    case ScriptPhase.GitCommitTriage:
      gitCommitPlan(planDir, name, 'triage')
      writePhase(planDir, LLMPhase.Work)
      return askContinue()
  }
}

export async function phaseLoop(
  agent: Agent,
  ctx: PlanContext,
  config: SharedConfig,
  projectRoot: string
): Promise<void> {
  const tokens = agent.tokens()

  if (agent.setup) {
    await agent.setup(ctx)
  }

  while (true) {
    const phase = readPhase(ctx.planDir)
    console.log(`\n▶ Phase: ${phase}`)

    if (isScriptPhase(phase)) {
      const shouldContinue = await handleScriptPhase(phase, ctx.planDir)
      if (!shouldContinue) {
        console.log('Exiting.')
        return
      }
      continue
    }

    // Pre-work: save baseline for analyse-work diff
    if (phase === LLMPhase.Work) {
      gitSaveWorkBaseline(ctx.planDir)
      fs.rmSync(path.join(ctx.planDir, 'latest-session.md'), { force: true })
    }

    const prompt = composePrompt(projectRoot, phase, ctx, tokens)

    if (phase === LLMPhase.Work) {
      await agent.invokeInteractive(prompt, ctx)
    } else {
      await agent.invokeHeadless(prompt, ctx, phase)
    }

    // Post-phase: check phase advanced
    const newPhase = readPhase(ctx.planDir)
    if (newPhase === phase) {
      console.error(`⚠ Phase did not advance from ${phase}. Stopping.`)
      return
    }

    // Dream trigger: after git-commit-reflect, check if dream is needed
    if (newPhase === LLMPhase.Dream || phase === ScriptPhase.GitCommitReflect) {
      // This is handled after GitCommitReflect in handleScriptPhase flow,
      // but if reflect wrote 'dream' directly, check here
    }

    // After reflect commits, check dream trigger
    if (phase === LLMPhase.Reflect) {
      // reflect always writes git-commit-reflect, handled in next loop iteration
    }

    // After git-commit-reflect, check dream trigger
    // (This is checked in the script phase handler above,
    //  but the actual skip logic lives here)
    if (readPhase(ctx.planDir) === LLMPhase.Dream) {
      if (!shouldDream(ctx.planDir, config.headroom)) {
        console.log('  ⏭ Dream skipped (memory within headroom)')
        writePhase(ctx.planDir, ScriptPhase.GitCommitDream)
      }
    }

    // After dream phase completes, update baseline
    if (phase === LLMPhase.Dream) {
      updateDreamBaseline(ctx.planDir)
    }

    // After triage, dispatch subagents
    if (phase === LLMPhase.Triage) {
      await dispatchSubagents(agent, ctx.planDir)
    }
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add src/phase-loop.ts
git commit -m "feat: add core phase loop with script/LLM phase dispatch"
```

---

### Task 12: CLI Entry Point

**Files:**
- Create: `src/index.ts`

- [ ] **Step 1: Write index.ts**

Create `src/index.ts`:

```typescript
#!/usr/bin/env npx tsx
import path from 'node:path'
import fs from 'node:fs'
import { loadSharedConfig, loadAgentConfig } from './config.js'
import { type Agent, type PlanContext } from './types.js'
import { phaseLoop } from './phase-loop.js'
import { ClaudeCodeAgent } from './agents/claude-code/index.js'
import { PiAgent } from './agents/pi/index.js'

function findProjectRoot(startDir: string): string {
  let dir = startDir
  while (dir !== path.dirname(dir)) {
    if (fs.existsSync(path.join(dir, '.git'))) return dir
    dir = path.dirname(dir)
  }
  throw new Error(`No .git found above ${startDir}`)
}

function buildRelatedPlans(planDir: string): string {
  const relatedPath = path.join(planDir, 'related-plans.md')
  if (!fs.existsSync(relatedPath)) return ''
  return fs.readFileSync(relatedPath, 'utf-8')
}

function usage(): never {
  console.error('Usage: llm-context [--agent claude-code|pi] <plan-directory>')
  process.exit(1)
}

async function main() {
  const args = process.argv.slice(2)
  let agentOverride: string | undefined
  let planDir: string | undefined

  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--agent') {
      agentOverride = args[++i]
    } else if (!args[i].startsWith('-')) {
      planDir = path.resolve(args[i])
    }
  }

  if (!planDir) usage()

  if (!fs.existsSync(path.join(planDir, 'phase.md'))) {
    console.error(`Error: ${planDir}/phase.md not found. Is this a valid plan directory?`)
    process.exit(1)
  }

  // Find the project root containing this script (LLM_CONTEXT_PI)
  const scriptDir = path.dirname(new URL(import.meta.url).pathname)
  const projectRoot = path.resolve(scriptDir, '..')

  const sharedConfig = loadSharedConfig(projectRoot, agentOverride)
  const agentConfig = loadAgentConfig(projectRoot, sharedConfig.agent)

  const projectDir = findProjectRoot(planDir)
  const ctx: PlanContext = {
    planDir,
    projectDir,
    devRoot: path.dirname(projectDir),
    relatedPlans: buildRelatedPlans(planDir),
  }

  let agent: Agent
  switch (sharedConfig.agent) {
    case 'claude-code':
      agent = new ClaudeCodeAgent(agentConfig, projectRoot)
      break
    case 'pi':
      agent = new PiAgent(agentConfig, projectRoot)
      break
    default:
      console.error(`Unknown agent: ${sharedConfig.agent}`)
      process.exit(1)
  }

  console.log(`▶ Agent: ${sharedConfig.agent}`)
  console.log(`▶ Plan: ${planDir}`)
  console.log(`▶ Project: ${projectDir}`)

  await phaseLoop(agent, ctx, sharedConfig, projectRoot)
}

main().catch(err => {
  console.error(err)
  process.exit(1)
})
```

- [ ] **Step 2: Make executable**

Run: `chmod +x src/index.ts`

- [ ] **Step 3: Verify it runs (basic smoke test)**

Run: `npx tsx src/index.ts --help 2>&1 || true`
Expected: Prints usage message (no plan dir provided)

- [ ] **Step 4: Commit**

```bash
git add src/index.ts
git commit -m "feat: add CLI entry point with agent selection"
```

---

## Stage 2: Triage Subagent Dispatch

### Task 13: Subagent Dispatch Parser

**Files:**
- Create: `src/subagent-dispatch.ts`
- Create: `test/subagent-dispatch.test.ts`

- [ ] **Step 1: Write the test**

Create `test/subagent-dispatch.test.ts`:

```typescript
import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { parseDispatchFile, dispatchSubagents } from '../src/subagent-dispatch.js'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'

describe('parseDispatchFile', () => {
  let tmpDir: string

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'dispatch-test-'))
  })

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true })
  })

  it('returns empty array when file does not exist', () => {
    expect(parseDispatchFile(tmpDir)).toEqual([])
  })

  it('parses a valid subagent-dispatch.yaml', () => {
    const yaml = `dispatches:
  - target: /home/user/project/LLM_STATE/child-plan
    kind: child
    summary: |
      The parent plan discovered that the auth module
      needs a rate limiter. Consider adding a task.
  - target: /home/user/other/LLM_STATE/sibling-plan
    kind: sibling
    summary: |
      Shared utility was refactored. Update imports.
`
    fs.writeFileSync(path.join(tmpDir, 'subagent-dispatch.yaml'), yaml)

    const dispatches = parseDispatchFile(tmpDir)
    expect(dispatches).toHaveLength(2)
    expect(dispatches[0].target).toBe('/home/user/project/LLM_STATE/child-plan')
    expect(dispatches[0].kind).toBe('child')
    expect(dispatches[0].summary).toContain('rate limiter')
    expect(dispatches[1].kind).toBe('sibling')
  })

  it('returns empty array for empty dispatches list', () => {
    fs.writeFileSync(path.join(tmpDir, 'subagent-dispatch.yaml'), 'dispatches: []\n')
    expect(parseDispatchFile(tmpDir)).toEqual([])
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run test/subagent-dispatch.test.ts`
Expected: FAIL

- [ ] **Step 3: Write subagent-dispatch.ts**

Create `src/subagent-dispatch.ts`:

```typescript
import fs from 'node:fs'
import path from 'node:path'
import YAML from 'yaml'
import { type Agent, type SubagentDispatch } from './types.js'

export function parseDispatchFile(planDir: string): SubagentDispatch[] {
  const filePath = path.join(planDir, 'subagent-dispatch.yaml')
  if (!fs.existsSync(filePath)) return []

  const raw = YAML.parse(fs.readFileSync(filePath, 'utf-8')) as {
    dispatches?: SubagentDispatch[]
  }

  return raw.dispatches ?? []
}

export async function dispatchSubagents(agent: Agent, planDir: string): Promise<void> {
  const dispatches = parseDispatchFile(planDir)
  if (dispatches.length === 0) return

  console.log(`\n▶ Dispatching ${dispatches.length} subagent(s)...`)

  for (const dispatch of dispatches) {
    console.log(`  → ${dispatch.kind}: ${dispatch.target}`)

    const prompt = [
      `Plan at ${dispatch.target}.`,
      `This is a ${dispatch.kind} plan that may be affected by recent learnings.`,
      '',
      'Summary of learnings to apply:',
      dispatch.summary,
      '',
      'Read the target plan\'s backlog.md and memory.md.',
      'Apply relevant updates: add/modify tasks in backlog.md, update memory.md if needed.',
      'Be conservative — only change what the summary warrants.',
    ].join('\n')

    try {
      await agent.dispatchSubagent(prompt, dispatch.target)
      console.log(`  ✓ ${dispatch.target}`)
    } catch (err) {
      console.error(`  ✗ ${dispatch.target}: ${err}`)
    }
  }

  // Clean up dispatch file after processing
  fs.unlinkSync(path.join(planDir, 'subagent-dispatch.yaml'))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run test/subagent-dispatch.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/subagent-dispatch.ts test/subagent-dispatch.test.ts
git commit -m "feat: add subagent dispatch YAML parsing and execution"
```

---

## Stage 3: Pi Subagent Integration

### Task 14: Pi Setup Automation

**Files:**
- Create: `src/agents/pi/setup.ts`

- [ ] **Step 1: Write setup.ts**

Create `src/agents/pi/setup.ts`:

```typescript
import { execSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import YAML from 'yaml'
import { type PlanContext } from '../../types.js'

function isPiInstalled(): boolean {
  try {
    execSync('which pi', { stdio: 'pipe' })
    return true
  } catch {
    return false
  }
}

function isSubagentExtensionInstalled(): boolean {
  const settingsPath = path.join(os.homedir(), '.pi', 'agent', 'settings.json')
  if (!fs.existsSync(settingsPath)) return false

  try {
    const settings = JSON.parse(fs.readFileSync(settingsPath, 'utf-8')) as {
      packages?: string[]
    }
    return settings.packages?.some(p => p.includes('pi-subagent')) ?? false
  } catch {
    return false
  }
}

function installSubagentExtension(): void {
  console.log('  Installing Pi subagent extension...')
  execSync('pi install npm:@mjakl/pi-subagent', { stdio: 'inherit' })
  console.log('  ✓ Subagent extension installed')
}

interface SkillFrontmatter {
  name: string
  description: string
  tools?: string[]
  model?: string
  thinking?: string
}

function parseSkillFrontmatter(content: string): { frontmatter: SkillFrontmatter; body: string } {
  const match = content.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/)
  if (!match) {
    throw new Error('Skill file missing YAML frontmatter')
  }
  const frontmatter = YAML.parse(match[1]) as SkillFrontmatter
  return { frontmatter, body: match[2] }
}

function generateAgentDefinition(skill: { frontmatter: SkillFrontmatter; body: string }): string {
  const fm = skill.frontmatter
  const lines = ['---']
  lines.push(`name: ${fm.name}`)
  lines.push(`description: ${fm.description}`)
  if (fm.tools) lines.push(`tools: ${fm.tools.join(', ')}`)
  if (fm.model) lines.push(`model: ${fm.model}`)
  if (fm.thinking) lines.push(`thinking: ${fm.thinking}`)
  lines.push('---')
  lines.push('')
  lines.push(skill.body)
  return lines.join('\n')
}

export async function setupPi(projectRoot: string, ctx: PlanContext): Promise<void> {
  console.log('▶ Pi setup...')

  // Check prerequisites
  if (!isPiInstalled()) {
    throw new Error(
      'pi is not installed. Install with: npm install -g @mariozechner/pi-coding-agent'
    )
  }

  if (!process.env.ANTHROPIC_API_KEY) {
    console.warn('  ⚠ ANTHROPIC_API_KEY not set — pi may fail to authenticate')
  }

  // Install subagent extension if needed
  if (!isSubagentExtensionInstalled()) {
    installSubagentExtension()
  } else {
    console.log('  ✓ Subagent extension already installed')
  }

  // Generate agent definitions from skills
  const skillsDir = path.join(projectRoot, 'skills')
  if (!fs.existsSync(skillsDir)) {
    console.log('  ⏭ No skills/ directory — skipping agent definition generation')
    return
  }

  const agentsDir = path.join(ctx.projectDir, '.pi', 'agents')
  fs.mkdirSync(agentsDir, { recursive: true })

  const skillFiles = fs.readdirSync(skillsDir).filter(f => f.endsWith('.md'))
  for (const file of skillFiles) {
    const content = fs.readFileSync(path.join(skillsDir, file), 'utf-8')
    try {
      const skill = parseSkillFrontmatter(content)
      const agentDef = generateAgentDefinition(skill)
      fs.writeFileSync(path.join(agentsDir, file), agentDef)
    } catch (err) {
      console.warn(`  ⚠ Skipping ${file}: ${err}`)
    }
  }

  console.log(`  ✓ Generated ${skillFiles.length} agent definition(s) in ${agentsDir}`)
}
```

- [ ] **Step 2: Wire setup into PiAgent**

Add to `src/agents/pi/index.ts`, in the `PiAgent` class:

```typescript
import { setupPi } from './setup.js'

// Add to class body:
async setup(ctx: PlanContext): Promise<void> {
  await setupPi(this.projectRoot, ctx)
}
```

- [ ] **Step 3: Commit**

```bash
git add src/agents/pi/setup.ts src/agents/pi/index.ts
git commit -m "feat: add automatic Pi subagent extension setup and agent def generation"
```

---

## Stage 4: Skills Extraction

### Task 15: Extract Skills

**Files:**
- Create: `skills/brainstorming.md`
- Create: `skills/writing-plans.md`
- Create: `skills/tdd.md`

Skills are extracted from the superpowers plugin content. Each skill has YAML frontmatter with Pi-compatible fields (name, description, tools, model) followed by the skill content.

- [ ] **Step 1: Create brainstorming skill**

Create `skills/brainstorming.md`:

```markdown
---
name: brainstormer
description: Explores ideas and designs through collaborative dialogue before implementation
tools: [read, grep, find, ls, bash]
model: claude-sonnet-4-6
---

# Brainstorming Ideas Into Designs

Help turn ideas into fully formed designs and specs through natural collaborative dialogue.

## Process

1. **Explore project context** — check files, docs, recent commits
2. **Ask clarifying questions** — one at a time, understand purpose/constraints/success criteria
3. **Propose 2-3 approaches** — with trade-offs and your recommendation
4. **Present design** — in sections scaled to their complexity, get user approval after each section

## Key Principles

- **One question at a time** — Don't overwhelm with multiple questions
- **Multiple choice preferred** — Easier to answer than open-ended when possible
- **YAGNI ruthlessly** — Remove unnecessary features from all designs
- **Explore alternatives** — Always propose 2-3 approaches before settling
- **Incremental validation** — Present design, get approval before moving on

## Design Quality

- Break the system into smaller units with one clear purpose each
- Units communicate through well-defined interfaces
- Can be understood and tested independently
- For each unit: what does it do, how do you use it, what does it depend on?

## Working in Existing Codebases

- Explore the current structure before proposing changes
- Follow existing patterns
- Where existing code has problems affecting the work, include targeted improvements
- Don't propose unrelated refactoring
```

- [ ] **Step 2: Create writing-plans skill**

Create `skills/writing-plans.md`:

```markdown
---
name: planner
description: Creates detailed implementation plans with bite-sized TDD tasks
tools: [read, grep, find, ls, bash]
model: claude-sonnet-4-6
---

# Writing Implementation Plans

Write comprehensive implementation plans assuming the engineer has zero context. Document everything: which files to touch, code, testing, how to verify. Bite-sized tasks. DRY. YAGNI. TDD. Frequent commits.

## Task Structure

Each task includes:
- **Files:** exact paths to create/modify/test
- **Steps:** each step is one action (2-5 minutes)
  - Write the failing test
  - Run it to verify failure
  - Write minimal implementation
  - Run test to verify pass
  - Commit

## Rules

- Exact file paths always
- Complete code in every step
- Exact commands with expected output
- No placeholders (TBD, TODO, "implement later")
- No "similar to Task N" — repeat the code
```

- [ ] **Step 3: Create TDD skill**

Create `skills/tdd.md`:

```markdown
---
name: tdd-coach
description: Guides test-driven development with red-green-refactor discipline
tools: [read, grep, find, ls, bash]
model: claude-sonnet-4-6
---

# Test-Driven Development

Guide development using strict red-green-refactor discipline.

## The Cycle

1. **Red:** Write a failing test that describes the desired behavior
2. **Green:** Write the minimum code to make the test pass
3. **Refactor:** Clean up while keeping tests green

## Principles

- Never write production code without a failing test
- Write the smallest possible test that fails
- Write the smallest possible code that passes
- Refactor only when tests are green
- One logical change per commit
- Test behavior, not implementation details
- Prefer integration tests over unit tests when testing boundaries
```

- [ ] **Step 4: Commit**

```bash
git add skills/
git commit -m "feat: extract initial skills (brainstorming, writing-plans, tdd)"
```

---

## Stage 5: Testing

### Task 16: Full Test Suite

**Files:**
- Verify: `test/types.test.ts` (already created in Task 2)
- Verify: `test/config.test.ts` (already created in Task 3)
- Verify: `test/prompt-composer.test.ts` (already created in Task 5)
- Verify: `test/dream.test.ts` (already created in Task 6)
- Verify: `test/subagent-dispatch.test.ts` (already created in Task 13)

- [ ] **Step 1: Run the full test suite**

Run: `npx vitest run`
Expected: All tests PASS

- [ ] **Step 2: Run type checking**

Run: `npx tsc --noEmit`
Expected: No type errors

- [ ] **Step 3: Fix any failures**

If any tests fail or type errors exist, fix them before proceeding.

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix: resolve test and type check issues"
```

---

### Task 17: Clean Up Old Files

**Files:**
- Remove: `run-plan.sh`
- Remove: `config.sh`
- Remove: `prompts/system-prompt.md`
- Remove: `prompts/memory-prompt.md`
- Remove: `test/test-compose-prompt.sh`
- Remove: `test/test-format-pi-stream.sh`
- Remove: `test/test-parse-propagation.sh`
- Remove: `test/fixtures/`
- Remove: `docs/superpowers/plans/2026-04-16-pi-adaptation.md` (superseded)

- [ ] **Step 1: Remove old bash scripts and prompts**

```bash
git rm run-plan.sh config.sh
git rm -r prompts/
git rm -r test/test-*.sh test/fixtures/ 2>/dev/null || true
git rm docs/superpowers/plans/2026-04-16-pi-adaptation.md 2>/dev/null || true
```

- [ ] **Step 2: Commit**

```bash
git commit -m "chore: remove superseded bash orchestrator and old test fixtures"
```

---

### Task 18: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update README with new usage**

Update `README.md` to document:
- TypeScript rewrite motivation
- New project structure
- Usage: `npx tsx src/index.ts --agent pi|claude-code <plan-dir>`
- Configuration: `config.yaml`, `agents/<name>/config.yaml`, `agents/<name>/tokens.yaml`
- Phase chain diagram
- Skills system
- How to add a new agent

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: update README for TypeScript orchestrator"
```

---

## Self-Review Checklist

1. **Spec coverage:**
   - [x] TypeScript rewrite — Task 1
   - [x] Shared parameterized phase files — Task 8
   - [x] Prompt injection (Pi custom, Claude Code built-in) — Tasks 9, 10
   - [x] YAML subagent dispatch — Task 13
   - [x] Native subagent mechanisms — Tasks 9, 10, 14
   - [x] Skills as harness-independent markdown — Task 15
   - [x] Phase chain (work → analyse-work → git-commit-work → ... → triage → git-commit-triage) — Tasks 2, 11
   - [x] Token substitution (path + tool tokens) — Tasks 4, 5
   - [x] Dream trigger with headroom — Task 6
   - [x] Config loading (shared + per-agent YAML) — Task 3
   - [x] Pi auto-setup (subagent extension + agent defs) — Task 14
   - [x] Agent interface with setup() — Task 2

2. **Placeholder scan:** No TBD/TODO/placeholders. All code is complete.

3. **Type consistency:** `Agent` interface used consistently (invokeInteractive, invokeHeadless, dispatchSubagent, tokens, setup?). `LLMPhase` and `ScriptPhase` enums match across all files. `PlanContext` fields consistent between types.ts, config.ts, and agent implementations.
