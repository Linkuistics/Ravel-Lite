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
