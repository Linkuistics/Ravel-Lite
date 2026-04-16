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
