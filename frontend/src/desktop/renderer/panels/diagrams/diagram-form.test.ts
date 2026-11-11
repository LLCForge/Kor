import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseParams, statusLabel } from './diagram-form.ts'

test('params and labels', () => {
  assert.deepEqual(parseParams(' scope=src  depth=2 bad k-y=x =y from=main '), { scope: 'src', depth: '2', from: 'main' })
  assert.deepEqual(parseParams(''), {})
  assert.equal(statusLabel('edited'), 'elle düzenlenmiş')
})
