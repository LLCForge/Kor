import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { ReviewFile } from '../../ipc-review.ts'
import { counts, initialChoices, reviewReducer, shownAfter, toAnswer } from './review-state.ts'

const files: ReviewFile[] = [
  { path: 'a.rs', before: 'a', after: 'A' },
  { path: 'b.rs', before: 'b', after: 'B' },
  { path: 'old.txt', before: 'bye', after: null },
]

test('everything accepted is the plain yes (tool runs as proposed)', () => {
  assert.deepEqual(toAnswer(files, initialChoices(files)), { approval: 'yes' })
})

test('reject all is the plain no', () => {
  const s = reviewReducer(initialChoices(files), { type: 'all', kind: 'reject' })
  assert.deepEqual(toAnswer(files, s), { approval: 'no' })
  assert.deepEqual(counts(files, s), { accepted: 0, rejected: 3, edited: 0 })
})

test('subset and edited content become per-file decisions', () => {
  let s = initialChoices(files)
  s = reviewReducer(s, { type: 'set', path: 'b.rs', choice: { kind: 'reject' } })
  s = reviewReducer(s, { type: 'set', path: 'a.rs', choice: { kind: 'edit', content: 'mine' } })
  assert.deepEqual(toAnswer(files, s), { approval: 'yes', files: { 'a.rs': { edit: 'mine' }, 'b.rs': 'reject', 'old.txt': 'accept' } })
  assert.equal(shownAfter(files[0]!, s['a.rs']), 'mine')
  assert.deepEqual(counts(files, s), { accepted: 1, rejected: 1, edited: 1 })
})

test('an edit that matches the proposal is a plain accept', () => {
  const s = reviewReducer(initialChoices(files), { type: 'set', path: 'a.rs', choice: { kind: 'edit', content: 'A' } })
  assert.deepEqual(toAnswer(files, s), { approval: 'yes' })
})
