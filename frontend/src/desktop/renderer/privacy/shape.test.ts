import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { PrivacyCards, PrivacyRow } from '../../ipc.gen.ts'
import { badge, providerOf, rowFor, subProcessor } from './shape.ts'

const row = (provider: string, over: Partial<PrivacyRow['card']> = {}, unmet: string[] = []): PrivacyRow => ({
  card: {
    provider,
    model: null,
    training: 'unknown',
    retention: 'unknown',
    region: 'unknown',
    euResidency: null,
    zdr: 'unknown',
    gateway: false,
    subProcessor: null,
    notes: [],
    sources: [],
    local: false,
    ...over,
  },
  unmet,
  text: { training: 'bilinmiyor', retention: 'bilinmiyor', region: 'bilinmiyor', zdr: 'bilinmiyor', subProcessor: '-', law: 'not' },
})

const cards: PrivacyCards = {
  require: ['no-training'],
  providers: [
    row('anthropic', { training: 'no', retention: { days: 30 }, zdr: 'required', sources: [{ url: 'https://x', retrieved: '2026-11-10', quote: 'q' }] }),
    row('openrouter', { gateway: true }, ['no-training']),
    row('local', { local: true, training: 'no', retention: { days: 0 }, region: 'local', zdr: 'not-needed' }),
  ],
}

test('model kimliğinden kart bulunur, ağ geçidinde gerçek işleyici çıkar', () => {
  assert.equal(providerOf('openrouter/anthropic/claude'), 'openrouter')
  assert.equal(providerOf('claude'), null)
  assert.equal(rowFor(cards, 'nope/x'), null)
  const or = rowFor(cards, 'openrouter/anthropic/claude')!
  assert.equal(subProcessor(or, 'openrouter/anthropic/claude'), 'anthropic')
  assert.equal(subProcessor(rowFor(cards, 'anthropic/claude')!, 'anthropic/claude'), null)
})

test('karşılanmayan şart soluklaştırır, bilinmeyen uydurulmaz, kaynak görünür', () => {
  const or = badge(rowFor(cards, 'openrouter/anthropic/claude')!, 'openrouter/anthropic/claude')
  assert.equal(or.dimmed, true)
  assert.equal(or.tone, 'unknown')
  assert.match(or.title, /bilinmiyor/)
  assert.match(or.title, /Alt işleyici: anthropic/)
  const an = badge(rowFor(cards, 'anthropic/claude')!, 'anthropic/claude')
  assert.equal(an.dimmed, false)
  assert.equal(an.tone, 'warn')
  assert.match(an.title, /Kaynak: https:\/\/x \(2026-11-10\)/)
  assert.equal(badge(rowFor(cards, 'local/qwen')!, 'local/qwen').tone, 'ok')
})

test('rozet yalnızca bilinen alanları gösterir, hepsi bilinmiyorsa boş kalır', () => {
  assert.equal(badge(rowFor(cards, 'openrouter/x/y')!, 'openrouter/x/y').label, '')
  assert.equal(badge(rowFor(cards, 'anthropic/claude')!, 'anthropic/claude').label, 'bilinmiyor · bilinmiyor')
})
