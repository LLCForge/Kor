import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { RuleFinding } from '../../ipc.gen.ts'
import { countFindings, findingLocation, sortFindings } from './findings.ts'

const f = (verdict: RuleFinding['verdict'], source: string, line: number, file: string | null = null, fileLine: number | null = null): RuleFinding => ({
  verdict, rule: 'r', source, line, file, fileLine, message: '', deterministic: true,
})

test('counts, locations and order', () => {
  const list = [f('applied', 'A.md', 2), f('violated', 'CLAUDE.md', 9, 'src/a.rs', 3), f('relevant', 'A.md', 1), f('violated', 'AGENTS.md', 1, 'b.ts')]
  assert.deepEqual(countFindings(list), { violated: 2, relevant: 1, applied: 1 })
  assert.equal(findingLocation(list[1]!), 'src/a.rs:3')
  assert.equal(findingLocation(list[3]!), 'b.ts')
  assert.equal(findingLocation(list[0]!), '')
  assert.deepEqual(sortFindings(list).map((x) => `${x.verdict}:${x.source}`), ['violated:AGENTS.md', 'violated:CLAUDE.md', 'relevant:A.md', 'applied:A.md'])
})
