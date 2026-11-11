import assert from 'node:assert/strict'
import { test } from 'node:test'
import { estimable, formatDuration, progressLine } from './estimate.ts'

test('drafts worth estimating', () => {
  assert.equal(estimable('selam'), false)
  assert.equal(estimable('/compact şimdi lütfen'), false)
  assert.equal(estimable('parser modülünü yeniden düzenle'), true)
})

test('durations and drift', () => {
  assert.equal(formatDuration(45), '45 sn')
  assert.equal(formatDuration(240), '4 dk')
  assert.equal(formatDuration(72), '1 dk 12 sn')
  assert.equal(formatDuration(3900), '1 sa 5 dk')
  assert.deepEqual(progressLine(72, 240), { text: '1 dk 12 sn / ≈4 dk', drift: false })
  assert.equal(progressLine(500, 240).drift, true)
})
