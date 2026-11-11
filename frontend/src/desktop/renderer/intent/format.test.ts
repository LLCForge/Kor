// node --test src/desktop/renderer/intent/format.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { IntentPlan } from '../../ipc-intent.ts'
import { extra, headline, interval, needsUser, refill, viaLabel } from './format.ts'

const plan = (over: Partial<IntentPlan>): IntentPlan => ({
  command: 'kor explain',
  argv: ['explain'],
  line: 'kor explain',
  slash: '/explain',
  desktop: null,
  name: 'Dosyayı açıkla',
  run: 'confirm',
  via: 'keyword',
  score: 0.9,
  params: {},
  missing: ['file'],
  usd: null,
  eta_s: null,
  ...over,
})

test('extra shows only what is known, always as an estimate', () => {
  assert.equal(extra(plan({})), '')
  assert.equal(extra(plan({ eta_s: 12 })), '≈12 sn')
  assert.equal(extra(plan({ eta_s: 5, usd: 0.0123 })), '≈5 sn, ≈$0.01')
})

test('labels and headlines', () => {
  assert.equal(viaLabel(plan({ via: 'embedding' })), 'yerel gömme')
  assert.equal(headline({ kind: 'pipeline', steps: [{ type: 'agent', value: 'x' }] }), '1 adım sırayla çalışacak')
  assert.equal(headline({ kind: 'confirm', plan: plan({}) }), 'Bu komut çalıştırılsın mı?')
  assert.ok(needsUser({ kind: 'choose', options: [] }))
  assert.ok(!needsUser({ kind: 'agent' }))
  assert.ok(!needsUser({ kind: 'run', plan: plan({}) }))
})

test('intervals read naturally', () => {
  assert.equal(interval(10080), 'her hafta')
  assert.equal(interval(1440), 'her gün')
  assert.equal(interval(120), 'her 2 saatte bir')
  assert.equal(interval(15), 'her 15 dakikada bir')
})

test('missing values are appended to the sentence', () => {
  assert.equal(refill(' bu dosyayı açıkla ', { file: ' src/a.rs ', other: '' }), 'bu dosyayı açıkla src/a.rs')
})
