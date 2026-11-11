// node --test src/desktop/renderer/panels/offline/shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { OfflineEvent } from '../../../ipc-offline.ts'
import { age, missingText, percent, reduceSteps, size, type StepRow } from './shape.ts'

const ev = (over: Partial<OfflineEvent>): OfflineEvent => ({
  step: 'model',
  name: 'model-qwen3-4b',
  state: 'start',
  done: 0,
  total: null,
  message: null,
  ...over,
})

test('sizes and ages', () => {
  assert.equal(size(2_497_280_256), '2.5 GB')
  assert.equal(size(24_930_349), '24.9 MB')
  assert.equal(size(null), '?')
  assert.equal(age(null), 'yok')
  assert.equal(age(120), 'bir saatten yeni')
  assert.equal(age(5 * 3600), '5 saat önce')
  assert.equal(age(72 * 3600), '3 gün önce')
  assert.equal(missingText(['runtime', 'model-file']), 'yerel çalıştırıcı yok, model indirilmedi')
})

test('progress events fold into one row per step', () => {
  let rows: StepRow[] = []
  rows = reduceSteps(rows, ev({}))
  rows = reduceSteps(rows, ev({ state: 'progress', done: 50, total: 200 }))
  assert.equal(rows.length, 1)
  assert.equal(percent(rows[0]!), 25)
  rows = reduceSteps(rows, ev({ state: 'done' }))
  assert.equal(rows[0]!.state, 'done')
  assert.equal(rows[0]!.total, 200)
  // A late progress event does not reopen a finished row.
  rows = reduceSteps(rows, ev({ state: 'progress', done: 10, total: 200 }))
  assert.equal(rows[0]!.state, 'done')
  rows = reduceSteps(rows, ev({ step: 'catalog', name: 'model-catalog', state: 'error', message: 'yok' }))
  assert.equal(rows.length, 2)
  assert.equal(rows[1]!.message, 'yok')
  assert.equal(percent({ done: 1, total: null }), null)
})
