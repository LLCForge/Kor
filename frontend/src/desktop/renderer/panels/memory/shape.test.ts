import assert from 'node:assert/strict'
import { test } from 'node:test'
import { baseName, parseRefs, projectChoices, recordDate } from './shape.ts'

test('parseRefs splits, trims backticks and dedupes', () => {
  assert.deepEqual(parseRefs(' `a.rs`, Auth::login\n\na.rs ,b.rs'), ['a.rs', 'Auth::login', 'b.rs'])
  assert.deepEqual(parseRefs(''), [])
  assert.equal(parseRefs(Array.from({ length: 80 }, (_, i) => `f${i}`).join(',')).length, 50)
})

test('projectChoices puts the open project first and dedupes paths', () => {
  const c = projectChoices('C:/w/kor', ['C:\\w\\kor', 'C:/w/other/'], ['/home/x/lib'])
  assert.deepEqual(
    c.map((x) => x.value),
    ['C:/w/kor', 'C:/w/other', '/home/x/lib'],
  )
  assert.equal(c[0]?.label, 'kor (bu proje)')
  assert.equal(projectChoices(null, [], []).length, 0)
})

test('baseName and recordDate', () => {
  assert.equal(baseName('C:\\a\\b\\'), 'b')
  assert.equal(recordDate(0), '')
  assert.equal(recordDate(1_762_732_800), '2025-11-10')
})
