// §23.4 kod bloğu başlığı: dil + dosya yolu (ADR 0053).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { parseFence } from './fence-info.ts'

test('dil:yol, title=, path= ve çıplak yol biçimleri', () => {
  assert.deepEqual(parseFence('ts:src/a.ts', null), { language: 'ts', path: 'src/a.ts' })
  assert.deepEqual(parseFence('rust', 'title="crates/x/src/lib.rs"'), { language: 'rust', path: 'crates/x/src/lib.rs' })
  assert.deepEqual(parseFence('py', 'path=app/main.py {1,3}'), { language: 'py', path: 'app/main.py' })
  assert.deepEqual(parseFence('go', 'cmd/main.go'), { language: 'go', path: 'cmd/main.go' })
})

test('yol yoksa ya da şüpheliyse null', () => {
  assert.deepEqual(parseFence('bash', '{1,3}'), { language: 'bash', path: null })
  assert.deepEqual(parseFence('', ''), { language: '', path: null })
  assert.equal(parseFence(`ts:${'a'.repeat(400)}.ts`, null).path, null)
})
