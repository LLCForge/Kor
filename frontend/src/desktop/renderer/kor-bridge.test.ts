// node --test src/desktop/renderer/kor-bridge.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { decodeBytes } from './kor-bridge.ts'

test('base64 ile gelen ikili veri Uint8Array olur, diğer değerler aynen döner', () => {
  const out = decodeBytes({ $bytes: btoa('PDF\u0000\u00ff') })
  assert.ok(out instanceof Uint8Array)
  assert.deepEqual([...(out as Uint8Array)], [80, 68, 70, 0, 255])
  assert.deepEqual(decodeBytes({ a: 1 }), { a: 1 })
  assert.equal(decodeBytes(null), null)
  assert.equal(decodeBytes('x'), 'x')
})

test('kor_bytes yanıtı: readFile metne, g21ReadDoc Uint8Array olarak çözülür (§32.2)', async () => {
  const { BINARY, decodeBinary } = await import('./kor-bridge.ts')
  assert.equal(BINARY.readFile, 'text')
  assert.equal(BINARY.g21ReadDoc, 'bytes')
  const utf8 = new TextEncoder().encode('merhaba ş')
  assert.equal(decodeBinary('text', utf8.buffer), 'merhaba ş')
  const raw = decodeBinary('bytes', new Uint8Array([0, 255]).buffer)
  assert.ok(raw instanceof Uint8Array)
  assert.deepEqual([...(raw as Uint8Array)], [0, 255])
})
