// node --test src/desktop/renderer/osint/plan.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { planRequest, sameRoot } from './plan.ts'

test('shell open switches project and attaches files; same project only attaches', () => {
  assert.deepEqual(planRequest({ kind: 'open', root: 'C:\\p', files: ['C:\\p\\a.rs'], origin: 'shell' }, null), [
    { do: 'open', root: 'C:\\p' },
    { do: 'attach', files: ['C:\\p\\a.rs'] },
  ])
  assert.deepEqual(planRequest({ kind: 'open', root: 'C:\\P\\', files: [], origin: 'shell' }, 'c:/p'), [])
})

test('links ask first and never send by themselves', () => {
  const open = planRequest({ kind: 'open', root: '/x', files: [], origin: 'link' }, null)
  assert.equal(open[0]?.do, 'confirm')
  const task = planRequest({ kind: 'task', prompt: 'rm -rf everything', root: null, files: [], quick: null, origin: 'link' }, '/x')
  assert.deepEqual(task, [{ do: 'newChat' }, { do: 'draft', text: 'rm -rf everything' }])
  assert.ok(!task.some((s) => s.do === 'send'))
  const resume = planRequest({ kind: 'resume', id: 'abc', origin: 'link' }, null)
  assert.deepEqual(resume.map((s) => s.do), ['confirm', 'resume'])
})

test('quick tasks from the file manager are sent in the right project', () => {
  const steps = planRequest({ kind: 'task', prompt: 'Projeyi açıkla', root: '/p', files: ['/p/a.rs'], quick: 'explain', origin: 'shell' }, '/q')
  assert.deepEqual(steps, [
    { do: 'open', root: '/p' },
    { do: 'attach', files: ['/p/a.rs'] },
    { do: 'send', text: 'Projeyi açıkla' },
  ])
  const same = planRequest({ kind: 'task', prompt: 'x', root: '/p', files: [], quick: 'bugs', origin: 'shell' }, '/p')
  assert.deepEqual(same, [{ do: 'newChat' }, { do: 'send', text: 'x' }])
})

test('edit, palette, new, running and terminal', () => {
  assert.deepEqual(planRequest({ kind: 'edit', files: ['/a', '/b'], origin: 'shell' }, null), [{ do: 'edit', files: ['/a', '/b'] }])
  assert.deepEqual(planRequest({ kind: 'edit', files: [], origin: 'shell' }, null), [])
  assert.deepEqual(planRequest({ kind: 'palette' }, null), [{ do: 'palette' }])
  assert.deepEqual(planRequest({ kind: 'new' }, null), [{ do: 'newChat' }])
  assert.deepEqual(planRequest({ kind: 'running' }, null), [{ do: 'running' }])
  assert.deepEqual(planRequest({ kind: 'terminal', root: '/p', files: [] }, null), [])
  assert.ok(sameRoot('/a/', '/A') && !sameRoot(null, '/a'))
})
