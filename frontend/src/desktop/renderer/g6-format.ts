import { uiLocale } from './ui-locale.ts'
/** 9200 → "9,2k", 142000 → "142k", 1000000 → "1M" */
export function formatTokens(n: number): string {
  const fmt = (v: number) => v.toLocaleString(uiLocale(), { maximumFractionDigits: v < 10 ? 1 : 0 })
  if (n >= 1_000_000) return `${fmt(n / 1_000_000)}M`
  if (n >= 1_000) return `${fmt(n / 1_000)}k`
  return String(Math.round(n))
}

export const percent = (part: number, whole: number) => (whole > 0 ? Math.min(100, Math.max(0, Math.round((part / whole) * 100))) : 0)
