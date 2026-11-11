import type { CatalogModel } from '../../core/model-catalog.ts'
import { uiLocale } from './ui-locale.ts'

export function formatUsd(n: number | null, digits = 2): string {
  if (n === null) return '—'
  return n.toLocaleString(uiLocale(), { style: 'currency', currency: 'USD', minimumFractionDigits: digits, maximumFractionDigits: Math.max(digits, 4) })
}

/** 1.000.000 → "1M", 256.000 → "256K" */
export function formatTokens(n: number | null): string {
  if (n === null) return '—'
  if (n >= 1_000_000) return `${Number((n / 1_000_000).toFixed(1)).toLocaleString(uiLocale())}M`
  if (n >= 1_000) return `${Math.round(n / 1_000).toLocaleString(uiLocale())}K`
  return n.toLocaleString(uiLocale())
}

export function formatScore(n: number | null | undefined): string {
  return n == null ? '—' : n.toLocaleString(uiLocale(), { maximumFractionDigits: 1 })
}

export function formatMs(ms: number | null): string {
  if (ms === null) return '—'
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toLocaleString(uiLocale(), { maximumFractionDigits: 1 })} sn`
}

export const TRAIT_LABELS: Readonly<Record<string, string>> = {
  default: 'Venice varsayılanı',
  function_calling_default: 'Araç çağırma varsayılanı',
  most_uncensored: 'En sansürsüz',
  default_vision: 'Görsel varsayılanı',
  most_intelligent: 'En zeki',
  default_reasoning: 'Akıl yürütme varsayılanı',
  default_code: 'Kodlama varsayılanı',
}

export function traitLabel(trait: string): string {
  return TRAIT_LABELS[trait] ?? trait
}

export function capabilityList(m: CatalogModel): string[] {
  return [
    m.supportsTools && 'Araç çağırma',
    m.vision && 'Görsel girdi',
    m.reasoning && 'Akıl yürütme',
    m.codeOptimized && 'Koda optimize',
    m.webSearch && 'Web araması',
  ].filter((x): x is string => typeof x === 'string')
}

export const PRIVACY_LABELS = {
  private: { label: 'Özel', desc: 'İstekler Venice altyapısında işlenir, saklanmaz.' },
  anonymized: { label: 'Anonimleştirilmiş', desc: 'Üçüncü taraf sağlayıcıya kimliğin olmadan iletilir; sağlayıcının kendi politikası geçerlidir.' },
} as const

/** Puan çubuğu yüzdesi: katalogdaki en yüksek değere göre göreli. */
export function relativePercent(value: number | null | undefined, max: number): number {
  if (value == null || max <= 0) return 0
  return Math.max(3, Math.min(100, Math.round((value / max) * 100)))
}
