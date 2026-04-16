import fs from 'node:fs'
import path from 'node:path'
import YAML from 'yaml'
import { LLMPhase, type SharedConfig, type AgentConfig } from './types.js'

export function loadSharedConfig(projectRoot: string, cliAgent?: string, dangerous?: boolean): SharedConfig {
  const configPath = path.join(projectRoot, 'config.yaml')
  const raw = YAML.parse(fs.readFileSync(configPath, 'utf-8')) as {
    headroom: number
    agent: string
  }
  return {
    headroom: raw.headroom,
    agent: cliAgent ?? raw.agent,
    dangerous: dangerous ?? false,
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
