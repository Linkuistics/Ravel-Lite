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
  orchestratorRoot: string
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
