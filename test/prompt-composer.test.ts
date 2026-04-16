import { describe, it, expect } from 'vitest'
import { substituteTokens } from '../src/prompt-composer.js'
import type { PlanContext } from '../src/types.js'

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
