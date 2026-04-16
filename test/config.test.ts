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
