import assert from 'node:assert/strict'
import { test } from 'node:test'
import { append, explain, summarize } from './traffic.ts'

test('traffic summaries and the bounded log', () => {
  assert.equal(summarize({ ms: 0, dir: 'out', text: '{"id":3,"method":"tools/call"}' }), '→ tools/call #3')
  assert.equal(summarize({ ms: 0, dir: 'in', text: '{"id":3,"result":{}}' }), '← #3 tamam')
  assert.match(summarize({ ms: 0, dir: 'in', text: '{"id":4,"error":{"code":-32602,"message":"bad"}}' }), /inputSchema/)
  assert.equal(summarize({ ms: 0, dir: 'stderr', text: 'boom' }), 'stderr: boom')
  assert.match(explain(-32050), /sunucu hatası/)
  let log = [] as ReturnType<typeof append>
  for (let i = 0; i < 5; i++) log = append(log, { ms: i, dir: 'note', text: String(i) }, 3)
  assert.deepEqual(log.map((t) => t.text), ['2', '3', '4'])
})
