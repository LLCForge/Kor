import assert from 'node:assert/strict'
import { test } from 'node:test'
import { EMPTY, memText, pendingConfirm, reduce, taskStateText } from './shape.ts'

test('olaylar dökümü kurar, tekrar oynatma iki kez saymaz', () => {
  let s = reduce(EMPTY, { type: 'attached', task: 'r-1', from: 0 })
  const evs = [
    { type: 'turnStart', seq: 0 },
    { type: 'text', delta: 'mer', seq: 1 },
    { type: 'text', delta: 'haba', seq: 2 },
    { type: 'tool', id: 'c1', name: 'bash', summary: './deploy.sh', seq: 3 },
    { type: 'confirm', id: 'c1', name: 'bash', summary: './deploy.sh', destructive: null, seq: 4 },
  ]
  for (const e of evs) s = reduce(s, e)
  for (const e of evs) s = reduce(s, e)
  assert.equal(s.task, 'r-1')
  assert.equal(s.running, true)
  assert.deepEqual(s.rows[0], { kind: 'text', text: 'merhaba' })
  assert.equal(s.rows.length, 3)
  assert.equal(pendingConfirm(s)?.id, 'c1')
  s = reduce(s, { type: 'confirmed', id: 'c1', approval: 'yes', seq: 5 })
  s = reduce(s, { type: 'toolResult', id: 'c1', result: 'ok', seq: 6 })
  s = reduce(s, { type: 'done', seq: 7 })
  assert.equal(pendingConfirm(s), null)
  assert.equal(s.running, false)
  assert.deepEqual(s.rows[1], { kind: 'tool', id: 'c1', name: 'bash', summary: './deploy.sh', result: 'ok' })
  // Başka göreve bağlanma dökümü sıfırlar.
  s = reduce(s, { type: 'attached', task: 'r-2' })
  assert.deepEqual(s.rows, [])
  s = reduce(s, { type: 'tasks', tasks: [{ id: 'r-2', state: 'idle', project: '/p', prompt: 'x', started: 1, updated: 1, events: 0 }] })
  assert.equal(s.tasks.length, 1)
})

test('metinler', () => {
  assert.equal(taskStateText('waiting'), 'onay bekliyor')
  assert.equal(memText(null), '—')
  assert.equal(memText({ total_mb: 10, used_mb: 4, available_mb: 6 }), '4 / 10 MB (6 MB boş)')
})
