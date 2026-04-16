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
