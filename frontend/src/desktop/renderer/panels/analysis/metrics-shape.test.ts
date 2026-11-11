// node --test src/desktop/renderer/panels/analysis/metrics-shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { DeadItem, Dimension } from '../../../ipc-metrics.ts'
import { deadWhy, formulaFor, growthBasis, metricValue, num, pruneNote, signed, testBasis } from './metrics-shape.ts'

test('numbers use one decimal and Turkish separators', () => {
  assert.equal(num(12), '12')
  assert.equal(num(2.25), '2,3')
  assert.equal(signed(1.5), '+1,5')
  assert.equal(signed(-2), '-2')
  assert.equal(metricValue('%', 40), '%40')
  assert.equal(metricValue('bool', 0), 'yok')
})

test('documentation formula depends on public items', () => {
  const d = (value: number): Dimension => ({ key: 'documentation', score: 50, grade: 'F', formula: 'x', metrics: [{ key: 'public_items', value, unit: 'count', source: 's' }], note: null })
  assert.match(formulaFor(d(0)), /public öğe yok/)
  assert.match(formulaFor(d(3)), /^80 ×/)
})

test('sources and reasons are spelled out', () => {
  assert.equal(testBasis({ kind: 'coverage', percent: 42, source: 'lcov.info' }), 'kapsam %42 (lcov.info)')
  assert.equal(testBasis({ kind: 'no_tests' }), 'test bulunamadı')
  assert.match(growthBasis({ kind: 'git', rev: 'abc' }), /abc/)
  const item: DeadItem = { id: 'a#x', kind: 'function', name: 'x', file: 'a.rs', line: 1, end: 2, confidence: 'high', reason: { kind: 'unreachable' }, unchanged_days: 200 }
  assert.match(deadWhy(item), /200 gündür/)
  assert.match(deadWhy({ ...item, reason: { kind: 'name_elsewhere', count: 3 } }), /3 kez/)
  assert.equal(pruneNote('git commit: boom'), 'git commit: boom')
  assert.match(pruneNote('prune-no-checker'), /denetleyici/)
})
