// [g14] Gözlemlenebilirlik: saf hesaplamalar (yüzdelik, araç/model istatistikleri, hata serisi, maliyet atfı,
// harcama tahmini, geri bildirime dayalı model önerisi, Prometheus metin biçimi, OTLP/JSON dönüşümü).
// Durum tutmaz; telemetry-g14.ts kayıtları üretir, burada özetlenir. Testler doğrudan içe aktarır.

export type SpanKind = 'turn' | 'model' | 'tool'
export type SpanStatus = 'ok' | 'error' | 'cancelled' | 'denied'

/** OpenTelemetry uyumlu iz parçası (zamanlar epoch ms; OTLP'ye çevrilirken nanosaniyeye büyütülür). */
export interface SpanRecord {
  traceId: string
  spanId: string
  parentSpanId: string | null
  kind: SpanKind
  name: string
  start: number
  end: number
  status: SpanStatus
  message?: string
  attrs: Record<string, string | number | boolean>
}

export type StepKind = 'wait' | 'thinking' | 'response' | 'tool'

export interface TimelineStep {
  kind: StepKind
  label: string
  start: number
  end: number
  status: SpanStatus
}

/** İstem token'larının kaynağa göre kırılımı (tahmin, gerçek prompt_tokens'a ölçeklenmiş). */
export interface TokenSplit {
  system: number
  toolDefs: number
  toolResults: number
  history: number
}

export interface ModelCallRecord {
  id: string
  traceId: string
  sessionId: string
  model: string
  provider: string
  start: number
  firstTokenAt: number | null
  end: number
  status: SpanStatus
  message?: string
  promptTokens: number
  completionTokens: number
  cachedTokens: number
  split: TokenSplit
}

export interface ToolCallRecord {
  id: string
  traceId: string
  sessionId: string
  name: string
  /** Maskeli, kısaltılmış özet (dosya yolu, komut) */
  summary: string
  start: number
  end: number
  status: SpanStatus
  slow: boolean
}

export interface TurnRecord {
  traceId: string
  spanId: string
  parentSpanId: string | null
  sessionId: string
  /** Alt ajan turuysa ajan tipi */
  subagent: string | null
  project: string
  model: string
  start: number
  end: number
  status: SpanStatus
  message?: string
  steps: TimelineStep[]
  tokens: TokenSplit & { completion: number; cached: number }
  costUsd: number | null
  modelCalls: number
  toolCalls: number
}

/** Maliyet defteri: model çağrısı başına bir satır (içerik yok, yalnızca sayılar ve adlar). */
export interface LedgerEntry {
  at: number
  sessionId: string
  traceId: string
  project: string
  model: string
  promptTokens: number
  completionTokens: number
  cachedTokens: number
  /** Kayıt anında fiyat biliniyorsa; yoksa özetlerken güncel fiyatla hesaplanır */
  costUsd: number | null
  tools: string[]
  tags: string[]
  ok: boolean
}

export interface FeedbackEntry {
  at: number
  traceId: string
  sessionId: string
  model: string
  /** 0: değerlendirme geri alındı */
  rating: 1 | -1 | 0
  costUsd: number | null
}

export interface Price {
  /** USD / 1M token */
  input: number
  output: number
}

export type PriceLookup = (model: string) => Price | null

// ---------- Yüzdelik ve gecikme ----------

/** Sıralı dizide en yakın sıra yöntemiyle yüzdelik (p: 0-1). Boş dizide 0. */
export function percentile(sorted: readonly number[], p: number): number {
  if (sorted.length === 0) return 0
  const rank = Math.ceil(p * sorted.length) - 1
  return sorted[Math.min(sorted.length - 1, Math.max(0, rank))] ?? 0
}

export interface LatencySummary {
  count: number
  p50: number
  p95: number
  max: number
  sum: number
}

export function summarizeLatency(durations: readonly number[]): LatencySummary {
  const sorted = [...durations].sort((a, b) => a - b)
  return {
    count: sorted.length,
    p50: percentile(sorted, 0.5),
    p95: percentile(sorted, 0.95),
    max: sorted.at(-1) ?? 0,
    sum: sorted.reduce((s, d) => s + d, 0),
  }
}

export interface ToolStat extends LatencySummary {
  name: string
  ok: number
  errors: number
  denied: number
  cancelled: number
  slow: number
  /** 0-1; reddedilen/iptal edilen çağrılar paydada yer almaz */
  successRate: number
}

export function toolStats(records: readonly ToolCallRecord[]): ToolStat[] {
  const byName = new Map<string, ToolCallRecord[]>()
  for (const r of records) byName.set(r.name, [...(byName.get(r.name) ?? []), r])
  return [...byName]
    .map(([name, list]) => {
      const count = (s: SpanStatus) => list.filter((r) => r.status === s).length
      const ok = count('ok')
      const errors = count('error')
      return {
        name,
        ...summarizeLatency(list.map((r) => r.end - r.start)),
        ok,
        errors,
        denied: count('denied'),
        cancelled: count('cancelled'),
        slow: list.filter((r) => r.slow).length,
        successRate: ok + errors === 0 ? 1 : ok / (ok + errors),
      }
    })
    .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name))
}

export interface ModelStat extends LatencySummary {
  model: string
  errors: number
  /** İlk token süresi (ms) medyanı; akış görülmediyse null */
  ttftP50: number | null
  promptTokens: number
  completionTokens: number
  cachedTokens: number
}

export function modelStats(records: readonly ModelCallRecord[]): ModelStat[] {
  const byModel = new Map<string, ModelCallRecord[]>()
  for (const r of records) byModel.set(r.model, [...(byModel.get(r.model) ?? []), r])
  return [...byModel]
    .map(([model, list]) => {
      const ttft = list.flatMap((r) => (r.firstTokenAt === null ? [] : [r.firstTokenAt - r.start])).sort((a, b) => a - b)
      return {
        model,
        ...summarizeLatency(list.map((r) => r.end - r.start)),
        errors: list.filter((r) => r.status === 'error').length,
        ttftP50: ttft.length > 0 ? percentile(ttft, 0.5) : null,
        promptTokens: list.reduce((s, r) => s + r.promptTokens, 0),
        completionTokens: list.reduce((s, r) => s + r.completionTokens, 0),
        cachedTokens: list.reduce((s, r) => s + r.cachedTokens, 0),
      }
    })
    .sort((a, b) => b.count - a.count || a.model.localeCompare(b.model))
}

// ---------- Hata oranı serisi ----------

export interface ErrorBucket {
  start: number
  toolCalls: number
  toolErrors: number
  modelCalls: number
  modelErrors: number
}

/** Son `buckets` x `bucketMs` aralığını eşit kovalara böler; kayıt bitiş zamanına göre kovalanır. */
export function errorSeries(
  tools: readonly ToolCallRecord[],
  models: readonly ModelCallRecord[],
  now: number,
  bucketMs = 5 * 60_000,
  buckets = 12,
): ErrorBucket[] {
  const first = now - bucketMs * buckets
  const series = Array.from({ length: buckets }, (_, i) => ({ start: first + i * bucketMs, toolCalls: 0, toolErrors: 0, modelCalls: 0, modelErrors: 0 }))
  const slot = (t: number) => (t < first || t > now ? null : series[Math.min(buckets - 1, Math.floor((t - first) / bucketMs))])
  for (const r of tools) {
    const b = slot(r.end)
    if (!b) continue
    b.toolCalls++
    if (r.status === 'error') b.toolErrors++
  }
  for (const r of models) {
    const b = slot(r.end)
    if (!b) continue
    b.modelCalls++
    if (r.status === 'error') b.modelErrors++
  }
  return series
}

// ---------- Yavaş araç nedeni ----------

/** Eşik üstü süren aracın olası nedeni; kullanıcıya inceleme ipucu. */
export function slowToolHint(name: string, durationMs: number, p95Ms: number): string {
  const vsUsual = p95Ms > 0 && durationMs > p95Ms * 2 ? ' Bu araç için olağan sürenin (p95) iki katından uzun.' : ''
  const byTool: Record<string, string> = {
    bash: 'Komut uzun sürdü: derleme/test gibi uzun işleri arka planda (run_in_background) çalıştırmayı veya kapsamı daraltmayı düşünün.',
    web_fetch: 'Uzak sunucu yavaş yanıt verdi veya sayfa büyük; ağ gecikmesi olası.',
    web_search: 'Arama sağlayıcısı yavaş yanıt verdi; ağ gecikmesi olası.',
    agent: 'Alt ajan birden çok model ve araç çağrısı yaptı; alt ajanın izini İzler sekmesinde inceleyin.',
    grep: 'Arama çok sayıda dosyayı taradı; yol veya dosya deseniyle daraltın.',
    glob: 'Desen çok geniş bir ağacı taradı; .gitignore dışı büyük klasörler olabilir.',
  }
  return `${byTool[name] ?? 'Araç beklenenden uzun sürdü; girdi boyutunu ve dış bağımlılıkları kontrol edin.'}${vsUsual}`
}

// ---------- Maliyet ----------

export function costOf(model: string, promptTokens: number, completionTokens: number, price: PriceLookup): number | null {
  const p = price(model)
  return p ? (promptTokens * p.input + completionTokens * p.output) / 1e6 : null
}

export function entryCost(e: LedgerEntry, price: PriceLookup): number {
  return e.costUsd ?? costOf(e.model, e.promptTokens, e.completionTokens, price) ?? 0
}

export interface CostSlice {
  key: string
  usd: number
  tokens: number
  calls: number
}

export interface CostAttribution {
  totalUsd: number
  /** Fiyatı bilinmeyen modellerin çağrı sayısı (maliyet 0 sayıldı) */
  unpricedCalls: number
  byModel: CostSlice[]
  byTool: CostSlice[]
  byProject: CostSlice[]
  byTag: CostSlice[]
  byDay: CostSlice[]
}

const dayKey = (t: number) => {
  const d = new Date(t)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

/**
 * Araç ekseni: bir model çağrısının maliyeti, o yanıtın istediği araçlara eşit bölünür
 * (araç istemeyen yanıtlar "(yanıt)"). Etiket ekseninde bir çağrı her etiketine tam sayılır, toplamla karşılaştırılmaz.
 */
export function attributeCost(entries: readonly LedgerEntry[], price: PriceLookup, since: number): CostAttribution {
  const maps = { model: new Map<string, CostSlice>(), tool: new Map<string, CostSlice>(), project: new Map<string, CostSlice>(), tag: new Map<string, CostSlice>(), day: new Map<string, CostSlice>() }
  const add = (map: Map<string, CostSlice>, key: string, usd: number, tokens: number, calls: number) => {
    const s = map.get(key) ?? { key, usd: 0, tokens: 0, calls: 0 }
    s.usd += usd
    s.tokens += tokens
    s.calls += calls
    map.set(key, s)
  }
  let totalUsd = 0
  let unpricedCalls = 0
  for (const e of entries) {
    if (e.at < since) continue
    if (e.costUsd === null && price(e.model) === null) unpricedCalls++
    const usd = entryCost(e, price)
    const tokens = e.promptTokens + e.completionTokens
    totalUsd += usd
    add(maps.model, e.model, usd, tokens, 1)
    add(maps.project, e.project || '(klasörsüz)', usd, tokens, 1)
    add(maps.day, dayKey(e.at), usd, tokens, 1)
    const tools = e.tools.length > 0 ? e.tools : ['(yanıt)']
    for (const t of tools) add(maps.tool, t, usd / tools.length, tokens / tools.length, 1 / tools.length)
    for (const tag of e.tags.length > 0 ? e.tags : ['(etiketsiz)']) add(maps.tag, tag, usd, tokens, 1)
  }
  const sorted = (m: Map<string, CostSlice>) => [...m.values()].sort((a, b) => b.usd - a.usd || b.tokens - a.tokens)
  return {
    totalUsd,
    unpricedCalls,
    byModel: sorted(maps.model),
    byTool: sorted(maps.tool),
    byProject: sorted(maps.project),
    byTag: sorted(maps.tag),
    byDay: [...maps.day.values()].sort((a, b) => a.key.localeCompare(b.key)),
  }
}

// ---------- Harcama tahmini ----------

export interface SpendForecast {
  /** Son 7 günün (bugün dahil) günlük ortalaması */
  avg7: number
  avg30: number
  nextWeekUsd: number
  monthToDateUsd: number
  /** Ay sonu tahmini: ay başından bugüne gerçekleşen + kalan günler x avg7 */
  monthProjectionUsd: number
  daysLeftInMonth: number
  /** Veri olan gün sayısı; azsa tahmin güvenilmez */
  sampleDays: number
}

/** daily: YYYY-MM-DD → USD. Eksik günler 0 sayılır (harcama yok). */
export function forecastSpend(daily: ReadonlyArray<{ date: string; usd: number }>, today: Date): SpendForecast {
  const byDate = new Map<string, number>()
  for (const d of daily) byDate.set(d.date, (byDate.get(d.date) ?? 0) + d.usd)
  const sumDays = (n: number) => {
    let s = 0
    for (let i = 0; i < n; i++) s += byDate.get(dayKey(new Date(today.getFullYear(), today.getMonth(), today.getDate() - i).getTime())) ?? 0
    return s
  }
  const avg7 = sumDays(7) / 7
  const avg30 = sumDays(30) / 30
  const daysInMonth = new Date(today.getFullYear(), today.getMonth() + 1, 0).getDate()
  const monthToDateUsd = sumDays(today.getDate())
  const daysLeftInMonth = daysInMonth - today.getDate()
  const oldest = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 29).getTime()
  return {
    avg7,
    avg30,
    nextWeekUsd: avg7 * 7,
    monthToDateUsd,
    monthProjectionUsd: monthToDateUsd + daysLeftInMonth * avg7,
    daysLeftInMonth,
    sampleDays: [...byDate.entries()].filter(([date, usd]) => usd > 0 && new Date(`${date}T00:00:00`).getTime() >= oldest).length,
  }
}

// ---------- Geri bildirim ve model önerisi ----------

export interface ModelQuality {
  model: string
  likes: number
  dislikes: number
  /** Laplace düzeltmeli beğeni oranı: (beğeni + 1) / (toplam + 2) */
  score: number
  /** Değerlendirilen turların ortalama maliyeti; fiyat bilinmiyorsa null */
  avgTurnCostUsd: number | null
}

export interface ModelRecommendation {
  stats: ModelQuality[]
  pick: string | null
  reason: string
}

/** Aynı tur birden çok kez değerlendirildiyse son değerlendirme geçerlidir; 0 değerlendirmeyi kaldırır. */
export function latestRatings(feedback: readonly FeedbackEntry[]): FeedbackEntry[] {
  const byTrace = new Map<string, FeedbackEntry>()
  for (const f of [...feedback].sort((a, b) => a.at - b.at)) byTrace.set(f.traceId, f)
  return [...byTrace.values()].filter((f) => f.rating !== 0)
}

/**
 * En ucuz yeterince iyi model: en az `minRatings` değerlendirmesi olan modeller arasında en yüksek puanın
 * `tolerance` kadar altına kadar olanlardan tur başına en ucuzu. Maliyet bilinmiyorsa en yüksek puanlı.
 */
export function recommendModel(feedback: readonly FeedbackEntry[], minRatings = 3, tolerance = 0.1): ModelRecommendation {
  const byModel = new Map<string, FeedbackEntry[]>()
  for (const f of latestRatings(feedback)) byModel.set(f.model, [...(byModel.get(f.model) ?? []), f])
  const stats: ModelQuality[] = [...byModel]
    .map(([model, list]) => {
      const likes = list.filter((f) => f.rating === 1).length
      const dislikes = list.length - likes
      const costs = list.flatMap((f) => (f.costUsd === null ? [] : [f.costUsd]))
      return {
        model,
        likes,
        dislikes,
        score: (likes + 1) / (list.length + 2),
        avgTurnCostUsd: costs.length > 0 ? costs.reduce((s, c) => s + c, 0) / costs.length : null,
      }
    })
    .sort((a, b) => b.score - a.score || a.model.localeCompare(b.model))
  const eligible = stats.filter((s) => s.likes + s.dislikes >= minRatings)
  const best = eligible[0]
  if (!best) return { stats, pick: null, reason: `Öneri için en az bir modelde ${minRatings} değerlendirme gerekir.` }
  const near = eligible.filter((s) => s.score >= best.score - tolerance)
  const priced = near.filter((s) => s.avgTurnCostUsd !== null).sort((a, b) => (a.avgTurnCostUsd ?? 0) - (b.avgTurnCostUsd ?? 0))
  const pick = priced[0] ?? best
  const reason =
    pick.model === best.model
      ? `En yüksek beğeni puanı (${Math.round(best.score * 100)}%).`
      : `Beğeni puanı en iyiye yakın (${Math.round(pick.score * 100)}% / ${Math.round(best.score * 100)}%) ve tur başına daha ucuz.`
  return { stats, pick: pick.model, reason }
}

// ---------- Prometheus metin biçimi (0.0.4) ----------

export interface MetricSample {
  labels: Record<string, string>
  value: number
  /** Ad soneki (_sum, _count) */
  suffix?: string
}

export interface MetricFamily {
  name: string
  help: string
  type: 'counter' | 'gauge' | 'summary'
  samples: MetricSample[]
}

const escapeLabel = (v: string) => v.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n')
const escapeHelp = (v: string) => v.replace(/\\/g, '\\\\').replace(/\n/g, '\\n')
const formatValue = (v: number) => (Number.isFinite(v) ? String(v) : v > 0 ? '+Inf' : v < 0 ? '-Inf' : 'NaN')

export function renderPrometheus(families: readonly MetricFamily[]): string {
  const lines: string[] = []
  for (const f of families) {
    lines.push(`# HELP ${f.name} ${escapeHelp(f.help)}`, `# TYPE ${f.name} ${f.type}`)
    for (const s of f.samples) {
      const labels = Object.entries(s.labels)
        .map(([k, v]) => `${k}="${escapeLabel(v)}"`)
        .join(',')
      lines.push(`${f.name}${s.suffix ?? ''}${labels ? `{${labels}}` : ''} ${formatValue(s.value)}`)
    }
  }
  return `${lines.join('\n')}\n`
}

// ---------- OTLP/JSON ----------

type OtlpValue = { stringValue: string } | { intValue: string } | { doubleValue: number } | { boolValue: boolean }

function otlpValue(v: string | number | boolean): OtlpValue {
  if (typeof v === 'string') return { stringValue: v }
  if (typeof v === 'boolean') return { boolValue: v }
  return Number.isInteger(v) ? { intValue: String(v) } : { doubleValue: v }
}

const nanos = (ms: number) => `${BigInt(Math.round(ms * 1000)) * 1000n}`

const STATUS_CODE: Record<SpanStatus, number> = { ok: 1, error: 2, cancelled: 2, denied: 2 }

/** OTLP/JSON ExportTraceServiceRequest (kimlikler onaltılık, zamanlar nanosaniye dizesi). */
export function toOtlp(spans: readonly SpanRecord[], serviceVersion: string): unknown {
  return {
    resourceSpans: [
      {
        resource: {
          attributes: [
            { key: 'service.name', value: { stringValue: 'kor' } },
            { key: 'service.version', value: { stringValue: serviceVersion } },
          ],
        },
        scopeSpans: [
          {
            scope: { name: 'kor.g14', version: '1' },
            spans: spans.map((s) => ({
              traceId: s.traceId,
              spanId: s.spanId,
              ...(s.parentSpanId ? { parentSpanId: s.parentSpanId } : {}),
              name: s.name,
              // Model çağrısı dış sağlayıcıya istemci isteğidir; tur ve araçlar süreç içidir.
              kind: s.kind === 'model' ? 3 : 1,
              startTimeUnixNano: nanos(s.start),
              endTimeUnixNano: nanos(s.end),
              attributes: Object.entries(s.attrs).map(([key, value]) => ({ key, value: otlpValue(value) })),
              status: { code: STATUS_CODE[s.status], ...(s.message ? { message: s.message } : {}) },
            })),
          },
        ],
      },
    ],
  }
}

// ---------- Yerel uç nokta denetimleri ----------

const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]'])

/** OTLP gönderimi yalnızca bu makinedeki toplayıcıya: http(s), geri döngü adresi, kimlik bilgisi yok. */
export function isLocalEndpoint(value: string): boolean {
  try {
    const url = new URL(value)
    return (url.protocol === 'http:' || url.protocol === 'https:') && LOOPBACK_HOSTS.has(url.hostname) && !url.username && !url.password
  } catch {
    return false
  }
}

/** Prometheus uç noktası DNS yeniden bağlama saldırısına karşı yalnızca geri döngü Host başlığını kabul eder. */
export function isLoopbackHostHeader(host: string | undefined, port: number): boolean {
  return host === `127.0.0.1:${port}` || host === `localhost:${port}`
}

/** Tahmini kırılımı gerçek prompt_tokens'a orantılar; yuvarlama artığı geçmişe eklenir. */
export function scaleSplit(estimate: TokenSplit, actual: number): TokenSplit {
  const total = estimate.system + estimate.toolDefs + estimate.toolResults + estimate.history
  if (total <= 0 || actual <= 0) return { system: 0, toolDefs: 0, toolResults: 0, history: Math.max(0, actual) }
  const f = actual / total
  const system = Math.round(estimate.system * f)
  const toolDefs = Math.round(estimate.toolDefs * f)
  const toolResults = Math.round(estimate.toolResults * f)
  return { system, toolDefs, toolResults, history: Math.max(0, actual - system - toolDefs - toolResults) }
}
