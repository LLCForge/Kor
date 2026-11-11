// node --test src/desktop/renderer/panels/visual/shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { ReviewFile } from '../../../ipc-review.ts'
import { mb, toWrite } from './shape.ts'

const files: ReviewFile[] = [
  { path: 'src/A.tsx', before: null, after: 'a' },
  { path: 'src/B.tsx', before: 'old', after: 'b' },
  { path: 'src/C.css', before: null, after: 'c' },
]

test('onay yanıtı yazılacak dosyalara çevrilir', () => {
  assert.deepEqual(toWrite(files, 'no'), [])
  assert.deepEqual(toWrite(files, 'yes').map((f) => f.path), ['src/A.tsx', 'src/B.tsx', 'src/C.css'])
  assert.deepEqual(toWrite(files, 'always').length, 3)
  const mixed = toWrite(files, 'yes', { 'src/A.tsx': 'reject', 'src/B.tsx': { edit: 'B!' } })
  assert.deepEqual(mixed, [
    { path: 'src/B.tsx', content: 'B!' },
    { path: 'src/C.css', content: 'c' },
  ])
  assert.deepEqual(toWrite([{ path: 'x', before: 'x', after: null }], 'yes'), [])
})

test('boyut biçimi', () => {
  assert.equal(mb(147_951_465), '141.1 MB')
})
