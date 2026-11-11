import assert from 'node:assert/strict'
import { test } from 'node:test'
import { displayPath } from './display-path.ts'

test('Windows uzun yol öneki gösterimden atılır', () => {
  assert.equal(displayPath('\\\\?\\C:\\Users\\a b\\proje'), 'C:\\Users\\a b\\proje')
  assert.equal(displayPath('\\\\?\\UNC\\sunucu\\pay\\x'), '\\\\sunucu\\pay\\x')
  assert.equal(displayPath('C:\\x'), 'C:\\x')
  assert.equal(displayPath('/home/a'), '/home/a')
})
