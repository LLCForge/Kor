import assert from 'node:assert/strict'
import { test } from 'node:test'
import { reasonText } from './reasons.ts'

test('stale reasons read as text', () => {
  assert.equal(reasonText({ kind: 'file_missing', file: 'src/a.rs' }), 'src/a.rs silinmiş')
  assert.equal(reasonText({ kind: 'symbol_gone', symbol: 'connect' }), 'connect artık dosyalarında yok')
  assert.match(reasonText({ kind: 'project_missing', project: '/x' }), /\/x/)
  assert.match(reasonText({ kind: 'file_changed', file: 'b.rs', modified: 1_700_000_000 }), /^b\.rs /)
})
