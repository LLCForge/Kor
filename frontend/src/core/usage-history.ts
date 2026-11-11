// Venice kullanım geçmişi: GET /billing/usage-analytics (beta; günlük kovalar, en fazla 90 gün).
// Eski /billing/usage uç noktası 2026-09-16'da kaldırıldı (410), bu yüzden kullanılmıyor.
import { z } from 'zod'

const amount = z.number().finite()

export const usageAnalyticsSchema = z.object({
  byDate: z.array(z.object({ date: z.string().regex(/^\d{4}-\d{2}-\d{2}$/), USD: amount, DIEM: amount })).max(400),
  byModel: z
    .array(z.object({ modelName: z.string().max(300), totalUsd: amount, totalDiem: amount, totalUnits: amount, unitType: z.string().max(50).nullish() }))
    .max(1000),
})

export type UsagePeriod = 'day' | 'week'

export interface UsageBucket {
  /** Kovanın ilk günü, YYYY-MM-DD */
  start: string
  usd: number
  diem: number
}

export interface UsageHistory {
  period: UsagePeriod
  buckets: UsageBucket[]
  totalUsd: number
  totalDiem: number
  topModels: Array<{ name: string; usd: number; diem: number }>
}

/** Günlük grafikte 30 gün, haftalıkta 12 hafta (84 gün) istenir. */
export function lookbackDays(period: UsagePeriod): number {
  return period === 'day' ? 30 : 84
}

/** Günlük kovaları en yeni günden geriye 7'şerli gruplar; eksik günler sıfır sayılır. */
export function bucketUsage(byDate: ReadonlyArray<{ date: string; USD: number; DIEM: number }>, period: UsagePeriod): UsageBucket[] {
  const days = [...byDate].sort((a, b) => a.date.localeCompare(b.date)).map((d) => ({ start: d.date, usd: d.USD, diem: d.DIEM }))
  if (period === 'day') return days
  const weeks: UsageBucket[] = []
  for (let end = days.length; end > 0; end -= 7) {
    const chunk = days.slice(Math.max(0, end - 7), end)
    const first = chunk[0]
    if (!first) continue
    weeks.unshift({
      start: first.start,
      usd: chunk.reduce((s, d) => s + d.usd, 0),
      diem: chunk.reduce((s, d) => s + d.diem, 0),
    })
  }
  return weeks
}

export function summarizeUsage(json: unknown, period: UsagePeriod): UsageHistory {
  const data = usageAnalyticsSchema.parse(json)
  const buckets = bucketUsage(data.byDate, period)
  return {
    period,
    buckets,
    totalUsd: buckets.reduce((s, b) => s + b.usd, 0),
    totalDiem: buckets.reduce((s, b) => s + b.diem, 0),
    topModels: [...data.byModel]
      .sort((a, b) => b.totalUsd + b.totalDiem - (a.totalUsd + a.totalDiem))
      .slice(0, 5)
      .map((m) => ({ name: m.modelName, usd: m.totalUsd, diem: m.totalDiem })),
  }
}

/** OpenAI SDK ve fetch'in ağ hatası mesajları; bunlarla biten turlarda "Yeniden dene" gösterilir. */
export function isNetworkErrorMessage(message: string): boolean {
  return /connection error|request timed out|fetch failed|network|ENOTFOUND|ECONNREFUSED|ECONNRESET|ETIMEDOUT|EAI_AGAIN|ENETUNREACH|socket hang up/i.test(message)
}
