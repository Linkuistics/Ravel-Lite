import { describe, it, expect, beforeEach, afterEach } from 'vitest'
import { parseDispatchFile } from '../src/subagent-dispatch.js'
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
