// Maliyet / Bağlam / Bench panellerinin veri şekillendirme yardımcıları (saf; node --test ile sınanır).
import type { BenchGraphComparison, ContextBreakdown, CostRow, RouteCategory } from '../../../ipc-cost.ts'
import { formatTokens, formatUsd } from '../../g7-format.ts'
import { uiLocale, percentText } from '../../ui-locale.ts'

/** Sağlayıcı/katalog tutarı; tahmin "≈" ile işaretlenir (§5.2). */
export function money(usd: number | null, estimated: boolean, digits = 2): string {
  if (usd === null) return '?'
  return `${estimated ? '≈ ' : ''}${formatUsd(usd, digits)}`
}

/** Bir satırın tutarı; hiçbir isteğin fiyatı bilinmiyorsa "?". */
export function rowMoney(r: CostRow): string {
  return r.requests > 0 && r.unpriced === r.requests ? '?' : money(r.usd, r.estimated)
}

/** `sağlayıcı/model` satırlarını sağlayıcıya göre toplar; en pahalı önce. */
export function byProvider(rows: readonly CostRow[]): CostRow[] {
  const map = new Map<string, CostRow>()
  for (const r of rows) {
    const slash = r.model.indexOf('/')
    const provider = slash > 0 ? r.model.slice(0, slash) : r.model
    const acc = map.get(provider) ?? { model: provider, requests: 0, prompt: 0, completion: 0, usd: 0, estimated: false, unpriced: 0 }
    acc.requests += r.requests
    acc.prompt += r.prompt
    acc.completion += r.completion
    acc.usd += r.usd
    acc.estimated ||= r.estimated
    acc.unpriced += r.unpriced
    map.set(provider, acc)
  }
  return [...map.values()].sort((a, b) => b.usd - a.usd || a.model.localeCompare(b.model))
}

export type BudgetLevel = 'none' | 'ok' | 'alert' | 'over'

/** Harcamanın limite göre durumu; limit 0 = limit yok. */
export function budgetState(spent: number, limit: number, alertPercent: number): { level: BudgetLevel; pct: number | null } {
  if (!(limit > 0)) return { level: 'none', pct: null }
  const pct = Math.round((spent / limit) * 100)
  return { level: spent >= limit ? 'over' : pct >= alertPercent ? 'alert' : 'ok', pct }
}

/** Ayar dosyasına float olarak yazılacak tutar ("5" tamsayı olurdu); geçersizse null. */
export function decimalSetting(text: string): string | null {
  const t = text.trim()
  if (t === '') return null
  const n = Number(t)
  if (!Number.isFinite(n) || n < 0 || n > 1_000_000) return null
  return n.toFixed(2)
}

/** Yüzde ayarı (0–100 tamsayı); geçersizse null. */
export function percentSetting(text: string): string | null {
  const n = Number(text.trim())
  return text.trim() !== '' && Number.isInteger(n) && n >= 0 && n <= 100 ? String(n) : null
}

/** Model referansı: boş (= katalogdan seç) ya da `sağlayıcı/model` biçiminde, boşluksuz. */
export function validModelRef(text: string): boolean {
  const t = text.trim()
  return t === '' || (t.length <= 200 && /^[^\s/]+\/\S+$/.test(t))
}

/** Virgül/boşlukla ayrılmış model listesi, tekrarsız. */
export function parseModels(text: string): string[] {
  return [...new Set(text.split(/[\s,]+/).filter(Boolean))]
}

export const ROUTE_CATEGORIES: ReadonlyArray<{ key: RouteCategory; label: string }> = [
  { key: 'simple_edit', label: 'Basit düzenleme' },
  { key: 'complex_refactor', label: 'Karmaşık refactor' },
  { key: 'test_generation', label: 'Test üretimi' },
  { key: 'docs', label: 'Dokümantasyon' },
  { key: 'review', label: 'Review' },
  { key: 'planning', label: 'Planlama' },
]

export const routeLabel = (c: string | undefined): string => ROUTE_CATEGORIES.find((x) => x.key === c)?.label ?? 'Sınıfsız'

// ---- context ----

type PartKey = 'system' | 'rules' | 'repo_map' | 'memory' | 'tools' | 'active_files' | 'history' | 'tool_results'

export const CONTEXT_PARTS: ReadonlyArray<{ key: PartKey; label: string }> = [
  { key: 'system', label: 'Sistem istemi' },
  { key: 'rules', label: 'Kurallar/config' },
  { key: 'repo_map', label: 'Codebase index' },
  { key: 'memory', label: 'Hafıza' },
  { key: 'tools', label: 'Araç tanımları' },
  { key: 'active_files', label: 'Aktif dosyalar' },
  { key: 'history', label: 'Konuşma geçmişi' },
  { key: 'tool_results', label: 'Araç çıktıları' },
]

export interface ContextPart {
  key: PartKey
  label: string
  tokens: number
  /** Pencerenin yüzdesi (pencere bilinmiyorsa null) */
  pct: number | null
  /** Uyarı eşiğini geçti */
  over: boolean
}

/** Son isteğin kategori dökümü, eşik işaretli (kor_agent::context::Breakdown::dominant ile aynı kural). */
export function contextParts(b: ContextBreakdown, warnPercent: number): ContextPart[] {
  return CONTEXT_PARTS.map(({ key, label }) => {
    const tokens = b[key]
    const pct = b.window > 0 ? Math.floor((tokens * 100) / b.window) : null
    return { key, label, tokens, pct, over: warnPercent > 0 && b.window > 0 && tokens * 100 > warnPercent * b.window }
  })
}

export const contextTotal = (b: ContextBreakdown): number => CONTEXT_PARTS.reduce((s, p) => s + b[p.key], 0)

/** Zaman içindeki doluluk (pencerenin yüzdesi); son `max` istek, eskiden yeniye. */
export function contextSeries(history: readonly ContextBreakdown[], max = 60): Array<{ label: string; value: number | null; title: string }> {
  return history.slice(-max).map((b) => {
    const total = contextTotal(b)
    const time = new Date(b.ts * 1000).toLocaleTimeString(uiLocale(), { hour: '2-digit', minute: '2-digit' })
    const value = b.window > 0 ? Math.round((total * 100) / b.window) : null
    return { label: time, value, title: `${time} · ${b.model} · ≈ ${formatTokens(total)}${b.window > 0 ? ` / ${formatTokens(b.window)} (%${value})` : ''}` }
  })
}

// ---- bench ----

export interface GraphMetric {
  label: string
  off: number | null
  on: number | null
  offText: string
  onText: string
  /** on − off yüzde farkı metni (null: hesaplanamaz) */
  delta: string | null
}

const signed = (n: number, unit: string) => `${n > 0 ? '+' : ''}${n.toFixed(1)}${unit}`

/** `--graph both` karşılaştırması: token, maliyet, geçme oranı. */
export function graphMetrics(g: BenchGraphComparison): GraphMetric[] {
  const tokens = (s: BenchGraphComparison['on']) => s.input_tokens + s.output_tokens
  return [
    {
      label: 'Token',
      off: tokens(g.off),
      on: tokens(g.on),
      offText: formatTokens(tokens(g.off)),
      onText: formatTokens(tokens(g.on)),
      delta: g.token_delta_pct === null ? null : signed(g.token_delta_pct, '%'),
    },
    {
      label: 'Maliyet',
      off: g.off.cost_usd,
      on: g.on.cost_usd,
      offText: money(g.off.cost_usd, g.off.cost_estimated, 4),
      onText: money(g.on.cost_usd, g.on.cost_estimated, 4),
      delta: g.cost_delta_pct === null ? null : signed(g.cost_delta_pct, '%'),
    },
    {
      label: 'Geçme oranı',
      off: g.off.pass_rate * 100,
      on: g.on.pass_rate * 100,
      offText: percentText(Math.round(g.off.pass_rate * 100)),
      onText: percentText(Math.round(g.on.pass_rate * 100)),
      delta: signed(g.pass_delta_pp, ' puan'),
    },
  ]
}
