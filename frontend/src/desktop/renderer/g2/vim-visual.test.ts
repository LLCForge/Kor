// node --test src/desktop/renderer/g2/vim-visual.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { initialVimState, vimKey, type VimState } from './vim.ts'

const normal: VimState = { mode: 'normal', pending: '' }

test('visual mode selects, yanks, deletes and pastes', () => {
  const t = 'hello world'
  let r = vimKey(normal, 'v', t, 0)
  assert.equal(r.state.mode, 'visual')
  assert.deepEqual(r.selection, [0, 1])
  r = vimKey(r.state, 'w', t, r.cursor)
  assert.deepEqual(r.selection, [0, 7])
  const y = vimKey(r.state, 'y', t, r.cursor)
  assert.equal(y.state.mode, 'normal')
  assert.equal(y.state.register, 'hello w')
  const p = vimKey(y.state, 'p', t, 10)
  assert.equal(p.text, 'hello worldhello w')
  r = vimKey(vimKey(normal, 'v', t, 6).state, 'l', t, 6)
  const d = vimKey(r.state, 'd', t, r.cursor)
  assert.equal(d.text, 'hello rld')
  assert.equal(d.state.register, 'wo')
  assert.equal(vimKey(vimKey(normal, 'v', t, 2).state, 'Escape', t, 2).state.mode, 'normal')
})

test('dd keeps the line in the register and : maps to slash commands', () => {
  const dd = vimKey(vimKey(normal, 'd', 'a\nb', 0).state, 'd', 'a\nb', 0)
  assert.equal(dd.text, 'b')
  assert.equal(dd.state.register, 'a\n')
  const c = vimKey(normal, ':', '', 0)
  assert.equal(c.text, '/')
  assert.equal(c.state.mode, 'insert')
  assert.equal(vimKey(normal, ':', 'compact', 0).text, '/compact')
  assert.equal(vimKey(initialVimState, 'x', 'abc', 1).handled, false, 'insert mode types')
})
