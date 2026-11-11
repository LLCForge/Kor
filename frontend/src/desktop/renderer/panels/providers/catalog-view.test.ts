// node --test src/desktop/renderer/panels/providers/catalog-view.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { CatalogEntry } from '../../../ipc-providers.ts'
import { ago, compare, formatBytes, groupCatalog, maxOf, monogram, providerOf } from './catalog-view.ts'

function m(model_id: string, extra: Partial<CatalogEntry> = {}): CatalogEntry {
  return {
    model_id,
    provider: null,
    aa_intelligence_index: null,
    aa_coding_index: null,
    aa_math_index: null,
    speed_tps: null,
    ttft_ms: null,
    input_price_per_m: null,
    output_price_per_m: null,
    context: null,
    tool_calling: null,
    vision: null,
    license: null,
    aa_fetched: null,
    updated_at: null,
    source: {},
    ...extra,
  }
}

test('provider comes from field, then id prefix, then fallback', () => {
  assert.equal(providerOf(m('x', { provider: 'Anthropic' })), 'anthropic')
  assert.equal(providerOf(m('openai/gpt-5')), 'openai')
  assert.equal(providerOf(m('solo')), 'diğer')
})

test('monogram uses first letter, Turkish upper case', () => {
  assert.equal(monogram('ideogram'), 'İ')
  assert.equal(monogram('-x'), 'X')
  assert.equal(monogram(''), '?')
})

test('missing scores sort last; price sorts ascending', () => {
  const list = [m('a', { aa_intelligence_index: 10 }), m('b'), m('c', { aa_intelligence_index: 50 })]
  assert.deepEqual([...list].sort(compare('intelligence')).map((x) => x.model_id), ['c', 'a', 'b'])
  const priced = [m('a', { input_price_per_m: 3 }), m('b'), m('c', { input_price_per_m: 0.5 })]
  assert.deepEqual([...priced].sort(compare('priceIn')).map((x) => x.model_id), ['c', 'a', 'b'])
})

test('groups by provider in best-first order and filters by every word', () => {
  const list = [
    m('openai/gpt-5', { aa_intelligence_index: 60 }),
    m('anthropic/claude-opus', { aa_intelligence_index: 70 }),
    m('openai/gpt-5-mini', { aa_intelligence_index: 40 }),
  ]
  const g = groupCatalog(list, '', 'intelligence')
  assert.deepEqual(g.map((x) => x.provider), ['anthropic', 'openai'])
  assert.deepEqual(g[1]?.models.map((x) => x.model_id), ['openai/gpt-5', 'openai/gpt-5-mini'])
  assert.deepEqual(groupCatalog(list, 'openai mini', 'intelligence').flatMap((x) => x.models.map((y) => y.model_id)), ['openai/gpt-5-mini'])
  assert.equal(groupCatalog(list, 'yok', 'intelligence').length, 0)
})

test('maxOf ignores nulls', () => {
  assert.equal(maxOf([m('a', { aa_coding_index: 4 }), m('b')], (x) => x.aa_coding_index), 4)
  assert.equal(maxOf([], (x) => x.aa_coding_index), 0)
})

test('formatBytes and ago', () => {
  assert.equal(formatBytes(512), '512 B')
  assert.equal(formatBytes(1536 * 1024), '1,5 MB')
  assert.equal(formatBytes(null), '—')
  assert.equal(ago(1000, 1_030_000), 'az önce')
  assert.equal(ago(1000, 1_000_000 + 7_200_000), '2 sa önce')
})
