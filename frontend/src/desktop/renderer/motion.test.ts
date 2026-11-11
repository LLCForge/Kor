import { test } from 'node:test'
import assert from 'node:assert/strict'
import { isStill } from './motion.ts'

const base = { pref: 'system', osReduce: false, lowPower: false, still: false } as const

test('Sistem kipinde işletim sisteminin isteği izlenir', () => {
  assert.equal(isStill(base), false)
  assert.equal(isStill({ ...base, osReduce: true }), true)
})

test('"Açık" işletim sisteminin hareketi azalt isteğini geçersiz kılar', () => {
  assert.equal(isStill({ ...base, pref: 'on', osReduce: true }), false)
})

test('"Kapalı", düşük güç ve okunabilirlik ayarı her zaman durdurur', () => {
  assert.equal(isStill({ ...base, pref: 'off' }), true)
  assert.equal(isStill({ ...base, pref: 'on', lowPower: true }), true)
  assert.equal(isStill({ ...base, pref: 'on', still: true }), true)
})
