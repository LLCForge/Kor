// node --test src/desktop/renderer/forge/format.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { findingLocation, severityCounts, type Finding } from '../../ipc-forge.ts'

const f = (severity: Finding['severity'], line: number | null = null): Finding => ({ file: 'a.rs', line, severity, rule: 'r', message: 'm', source: 'model' })

test('önem sayıları kritikten bilgiye sıralanır, boşlar atlanır', () => {
  assert.deepEqual(severityCounts([f('low'), f('critical'), f('low')]), [['critical', 1], ['low', 2]])
  assert.deepEqual(severityCounts([]), [])
})

test('konum satır varsa dosya:satır olur', () => {
  assert.equal(findingLocation(f('info', 3)), 'a.rs:3')
  assert.equal(findingLocation(f('info')), 'a.rs')
})
