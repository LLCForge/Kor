import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { ForkNode } from '../../../ipc-ek14.ts'
import { acceptedFiles, flatten } from './merge.ts'

const files = [
  { path: 'a.rs', before: 'x', after: 'y' },
  { path: 'b.rs', before: null, after: 'new' },
  { path: 'c.rs', before: 'old', after: null },
]

test('review answers become merge writes', () => {
  assert.deepEqual(acceptedFiles(files, 'no'), [])
  assert.deepEqual(acceptedFiles(files, 'yes').map((f) => f.path), ['a.rs', 'b.rs', 'c.rs'])
  assert.equal(acceptedFiles(files, 'yes')[2]!.content, null)
  const d = acceptedFiles(files, 'yes', { 'a.rs': { edit: 'z' }, 'b.rs': 'reject' })
  assert.deepEqual(d, [{ path: 'a.rs', content: 'z' }, { path: 'c.rs', content: null }])
})

test('tree flattens with depth', () => {
  const n = (id: string, children: ForkNode[] = []): ForkNode => ({ id, title: id, at: null, model: null, merged: null, worktree: null, missing: false, children })
  assert.deepEqual(flatten([n('p', [n('a', [n('a1')]), n('b')])]).map((r) => `${r.depth}${r.node.id}`), ['0p', '1a', '2a1', '1b'])
})
