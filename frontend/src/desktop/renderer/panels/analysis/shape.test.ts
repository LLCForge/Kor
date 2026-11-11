// node --test src/desktop/renderer/panels/analysis/shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { Finding, Severity } from '../../../ipc-analysis.ts'
import { counts, filterFindings, location, SEVERITIES } from './shape.ts'

const f = (rule: string, severity: Severity, file = 'a.py', line = 1): Finding => ({ rule, source: 'kor', severity, file, line, message: `${rule} msg` })

const list: Finding[] = [f('kor.sql', 'high', 'db.py', 4), f('github-pat', 'critical'), f('kor.pii.identifier', 'info', 'm.py', 2)]

test('counts per severity', () => {
  assert.deepEqual(counts(list), { critical: 1, high: 1, medium: 0, low: 0, info: 1 })
})

test('filters by severity and text', () => {
  const all = new Set<Severity>(SEVERITIES)
  assert.equal(filterFindings(list, all, '').length, 3)
  assert.deepEqual(filterFindings(list, new Set<Severity>(['high', 'critical']), '').map((x) => x.rule), ['kor.sql', 'github-pat'])
  assert.deepEqual(filterFindings(list, all, 'DB.PY').map((x) => x.rule), ['kor.sql'])
  assert.equal(filterFindings(list, new Set<Severity>(), '').length, 0)
})

test('location', () => {
  assert.equal(location(f('r', 'low', 'src/x.rs', 9)), 'src/x.rs:9')
  assert.equal(location({ ...f('r', 'low', 'x', 3), column: 5 }), 'x:3:5')
})
