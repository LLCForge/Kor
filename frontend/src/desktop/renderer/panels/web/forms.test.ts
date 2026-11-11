import assert from 'node:assert/strict'
import { test } from 'node:test'
import { buildArgs, missing, resultFiles, resultText, splitList } from './forms.ts'

test('buildArgs converts types and skips empty fields', () => {
  const fields = [
    { name: 'name', label: 'Ad', required: true },
    { name: 'rate', label: 'Oran', type: 'number' as const },
    { name: 'only', label: 'Yalnız', list: true },
    { name: 'live', label: 'Canlı', type: 'checkbox' as const },
    { name: 'note', label: 'Not' },
  ]
  assert.deepEqual(buildArgs(fields, { name: ' prod ', rate: '20,5', only: 'a, b\nc', live: true, note: '  ' }), {
    name: 'prod',
    rate: 20.5,
    only: ['a', 'b', 'c'],
    live: true,
  })
  assert.deepEqual(missing(fields, { name: '' }), ['Ad'])
  assert.deepEqual(splitList(' x ,,y\n'), ['x', 'y'])
})

test('resultText prefers markdown and falls back to JSON', () => {
  assert.equal(resultText({ markdown: '# A' }), '# A')
  assert.equal(resultText({ text: 'map' }), 'map')
  assert.equal(resultText([1, 2]), '[\n  1,\n  2\n]')
  assert.equal(resultText(null), '')
})

test('resultFiles collects saved paths', () => {
  assert.deepEqual(resultFiles({ files: ['C:\\a\\b.pdf'], saved: '/x/y.csv' }), ['C:\\a\\b.pdf', '/x/y.csv'])
  assert.deepEqual(resultFiles(['/p/q.php']), ['/p/q.php'])
  assert.deepEqual(resultFiles(['a', 'b']), [])
})
