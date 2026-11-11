import assert from 'node:assert/strict'
import { test } from 'node:test'
import { messageNumber } from './fork-match.ts'

test('maps a chat message to its transcript number', () => {
  const msgs = [
    { n: 1, role: 'user', text: 'fix the parser' },
    { n: 2, role: 'assistant', text: 'Looking at it' },
    { n: 3, role: 'tool', text: 'ok' },
    { n: 4, role: 'assistant', text: 'Done: parser fixed' },
    { n: 5, role: 'user', text: 'again' },
    { n: 6, role: 'user', text: 'again' },
  ]
  assert.equal(messageNumber(msgs, 'user', 'fix the parser', 0), 1)
  assert.equal(messageNumber(msgs, 'assistant', 'Done: parser fixed\n\nmore text', 0), 4)
  assert.equal(messageNumber(msgs, 'user', 'again', 1), 6)
  assert.equal(messageNumber(msgs, 'user', 'missing', 0), null)
  assert.equal(messageNumber(msgs, 'user', '   ', 0), null)
})
