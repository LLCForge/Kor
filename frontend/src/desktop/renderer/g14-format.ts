import { uiLocale } from './ui-locale.ts'
// [g14] Gözlem panoları için biçimlendiriciler (saf; testlerde doğrudan kullanılır).

const compact = new Intl.NumberFormat(uiLocale(), { notation: 'compact', maximumFractionDigits: 1 })
const integer = new Intl.NumberFormat(uiLocale())
const percent = new Intl.NumberFormat(uiLocale(), { style: 'percent', maximumFractionDigits: 1 })
const clock = new Intl.DateTimeFormat(uiLocale(), { hour: '2-digit', minute: '2-digit', second: '2-digit' })
const dayTime = new Intl.DateTimeFormat(uiLocale(), { day: '2-digit', month: 'short', hour: '2-digit', minute: '2-digit' })

export function formatMs(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '-'
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) return `${(ms / 1000).toLocaleString(uiLocale(), { maximumFractionDigits: 1 })} sn`
  const min = Math.floor(ms / 60_000)
  const sec = Math.round((ms % 60_000) / 1000)
  return sec > 0 ? `${min} dk ${sec} sn` : `${min} dk`
}

/** Küçük tutarlar için 4 ondalığa kadar; null: fiyat bilinmiyor. */
export function formatUsd(usd: number | null): string {
  if (usd === null) return '?'
  const digits = usd !== 0 && Math.abs(usd) < 0.01 ? 4 : 2
  return `$${usd.toLocaleString(uiLocale(), { minimumFractionDigits: 2, maximumFractionDigits: digits })}`
}

export const formatTokens = (n: number) => (n < 10_000 ? integer.format(Math.round(n)) : compact.format(n))
export const formatPercent = (ratio: number) => percent.format(ratio)
export const formatClock = (t: number) => clock.format(t)
export const formatDayTime = (t: number) => dayTime.format(t)
export const formatMb = (mb: number) => `${integer.format(Math.round(mb))} MB`
