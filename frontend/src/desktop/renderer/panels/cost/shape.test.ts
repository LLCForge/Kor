// node --test src/desktop/renderer/panels/cost/shape.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { BenchGraphComparison, BenchSummary, ContextBreakdown, CostRow } from '../../../ipc-cost.ts'
import { budgetState, byProvider, contextParts, contextSeries, decimalSetting, graphMetrics, money, parseModels, percentSetting, rowMoney, validModelRef } from './shape.ts'

const row = (model: string, usd: number, extra: Partial<CostRow> = {}): CostRow => ({
  model,
  requests: 1,
  prompt: 10,
  completion: 5,
  usd,
  estimated: false,
  unpriced: 0,
  ...extra,
})

test('tahmini tutar ≈ ile, fiyatsız satır ? ile gösterilir', () => {
  assert.ok(money(0.5, true).startsWith('≈ '))
  assert.ok(!money(0.5, false).includes('≈'))
  assert.equal(money(null, false), '?')
  assert.equal(rowMoney(row('a/b', 0, { unpriced: 1 })), '?')
})

test('sağlayıcı kırılımı modelleri toplar ve tahmin bayrağını korur', () => {
  const out = byProvider([row('anthropic/sonnet', 0.84), row('openrouter/qwen', 0.03, { estimated: true }), row('anthropic/haiku', 0.1, { requests: 3 })])
  assert.deepEqual(
    out.map((r) => [r.model, Number(r.usd.toFixed(2)), r.requests, r.estimated]),
    [
      ['anthropic', 0.94, 4, false],
      ['openrouter', 0.03, 1, true],
    ],
  )
})

test('bütçe durumu: limitsiz, normal, uyarı, aşım', () => {
  assert.deepEqual(budgetState(3, 0, 80), { level: 'none', pct: null })
  assert.deepEqual(budgetState(1, 5, 80), { level: 'ok', pct: 20 })
  assert.deepEqual(budgetState(4, 5, 80), { level: 'alert', pct: 80 })
  assert.deepEqual(budgetState(5, 5, 80), { level: 'over', pct: 100 })
})

test('ayar değerleri doğrulanır; tutar float olarak yazılır', () => {
  assert.equal(decimalSetting('5'), '5.00')
  assert.equal(decimalSetting('-1'), null)
  assert.equal(decimalSetting('abc'), null)
  assert.equal(percentSetting('80'), '80')
  assert.equal(percentSetting('80.5'), null)
  assert.equal(percentSetting('101'), null)
  assert.ok(validModelRef(''))
  assert.ok(validModelRef('groq/llama-3.3-70b'))
  assert.ok(!validModelRef('llama'))
  assert.ok(!validModelRef('a/b c'))
  assert.deepEqual(parseModels(' a/b, c/d  a/b\n'), ['a/b', 'c/d'])
})

const breakdown = (over: Partial<ContextBreakdown> = {}): ContextBreakdown => ({
  ts: 1_700_000_000,
  model: 'm/x',
  window: 200_000,
  estimated: true,
  system: 5_000,
  rules: 30_000,
  repo_map: 45_000,
  memory: 0,
  tools: 0,
  active_files: 23_000,
  history: 89_000,
  tool_results: 0,
  warning: null,
  ...over,
})

test('bağlam dökümü eşiği geçen kategoriyi işaretler', () => {
  const parts = contextParts(breakdown(), 40)
  const history = parts.find((p) => p.key === 'history')
  assert.equal(history?.pct, 44)
  assert.equal(history?.over, true)
  assert.equal(parts.filter((p) => p.over).length, 1)
  assert.equal(contextParts(breakdown(), 0).some((p) => p.over), false)
  assert.equal(contextParts(breakdown({ window: 0 }), 40)[0]?.pct, null)
})

test('bağlam geçmişi pencere yüzdesine çevrilir, son N istek', () => {
  const series = contextSeries([breakdown({ window: 0 }), breakdown(), breakdown()], 2)
  assert.equal(series.length, 2)
  assert.equal(series[0]?.value, 96)
})

const summary = (over: Partial<BenchSummary>): BenchSummary => ({
  model: 'm/x',
  arm: null,
  tasks: 2,
  passed: 2,
  pass_rate: 1,
  wall_ms: 0,
  input_tokens: 0,
  output_tokens: 0,
  cost_usd: 0,
  cost_estimated: false,
  tool_calls_ok: 0,
  tool_calls_repaired: 0,
  tool_calls_failed: 0,
  tool_success_rate: null,
  emulated: 0,
  ...over,
})

test('graf açık/kapalı karşılaştırması token, maliyet ve geçme oranı verir', () => {
  const g: BenchGraphComparison = {
    off: summary({ arm: 'graph-off', input_tokens: 9000, output_tokens: 1000, cost_usd: 0.02 }),
    on: summary({ arm: 'graph-on', input_tokens: 4000, output_tokens: 500, cost_usd: null, pass_rate: 0.5 }),
    token_delta_pct: -55,
    cost_delta_pct: null,
    pass_delta_pp: -50,
    meets_acceptance: false,
  }
  const [tokens, cost, pass] = graphMetrics(g)
  assert.deepEqual([tokens?.off, tokens?.on, tokens?.delta], [10_000, 4_500, '-55.0%'])
  assert.equal(cost?.onText, '?')
  assert.equal(cost?.delta, null)
  assert.deepEqual([pass?.on, pass?.delta], [50, '-50.0 puan'])
})
