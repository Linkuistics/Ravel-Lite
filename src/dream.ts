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
