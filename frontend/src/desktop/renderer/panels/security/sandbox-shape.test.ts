import assert from 'node:assert/strict'
import { test } from 'node:test'
import { limitText, parseAllowlist, parseLimit, validAllow } from './sandbox-shape.ts'

test('izin listesi girdileri kor_sandbox::valid_allow ile aynı kuralı izler', () => {
  for (const ok of ['*', 'github.com:443', 'localhost:*', '[::1]:8080', '*.npmjs.org:443']) assert.ok(validAllow(ok), ok)
  for (const bad of ['github.com', 'a:0', 'a:70000', ':443', 'a b:1', 'a":1']) assert.ok(!validAllow(bad), bad)
})

test('liste satır ve virgülle ayrılır, tekrarlar ve boşluklar atılır', () => {
  const r = parseAllowlist('github.com:443\n\n registry.npmjs.org:443 , github.com:443\nnope')
  assert.deepEqual(r.entries, ['github.com:443', 'registry.npmjs.org:443'])
  assert.deepEqual(r.invalid, ['nope'])
})

test('sınır alanları tam sayı ve üst sınır ister', () => {
  assert.equal(parseLimit(' 4096 ', 1_048_576), 4096)
  assert.equal(parseLimit('0', 100), 0)
  assert.equal(parseLimit('101', 100), null)
  assert.equal(parseLimit('1.5', 100), null)
  assert.equal(parseLimit('-1', 100), null)
  assert.equal(limitText(0, ' MiB', true), 'sınır yok')
  assert.equal(limitText(100, '%', true, true), 'sınır yok')
  assert.match(limitText(90, '%', false, true), /uygulayamıyor/)
})
