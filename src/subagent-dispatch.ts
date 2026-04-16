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
