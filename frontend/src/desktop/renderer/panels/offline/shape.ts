// Çevrimdışı paneli için saf yardımcılar (node --test ile sınanır).
import type { OfflineEvent, OfflineMissing } from '../../../ipc-offline.ts'
import { agoIntl } from '../../ui-locale.ts'

export function size(bytes: number | null | undefined): string {
  if (bytes == null) return '?'
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`
  return `${(bytes / 1e6).toFixed(1)} MB`
}

export function age(secs: number | null): string {
  if (secs == null) return 'yok'
  const intl = agoIntl(secs)
  if (intl) return intl
  const h = Math.floor(secs / 3600)
  if (h < 1) return 'bir saatten yeni'
  if (h < 48) return `${h} saat önce`
  return `${Math.floor(h / 24)} gün önce`
}

const MISSING: Record<OfflineMissing, string> = {
  model: 'çevrimdışı model seçilmedi',
  runtime: 'yerel çalıştırıcı yok',
  'model-file': 'model indirilmedi',
}

export function missingText(m: readonly OfflineMissing[]): string {
  return m.map((x) => MISSING[x] ?? x).join(', ')
}

export interface StepRow {
  key: string
  step: OfflineEvent['step']
  name: string
  state: OfflineEvent['state']
  done: number
  total: number | null
  message: string | null
}

/** İlerleme olaylarını adım satırlarına indirger (aynı adım+ad tek satır). */
export function reduceSteps(rows: readonly StepRow[], e: OfflineEvent): StepRow[] {
  const key = `${e.step}:${e.name}`
  const next: StepRow = { key, step: e.step, name: e.name, state: e.state, done: e.done, total: e.total, message: e.message }
  const i = rows.findIndex((r) => r.key === key)
  if (i < 0) return [...rows, next]
  const prev = rows[i]!
  // Bitmiş ya da hatalı satır geç gelen ilerlemeyle geri dönmez.
  if ((prev.state === 'done' || prev.state === 'error') && e.state === 'progress') return [...rows]
  const merged: StepRow = {
    ...next,
    done: e.state === 'progress' ? e.done : prev.done,
    total: e.total ?? prev.total,
    message: e.message ?? (e.state === 'progress' ? prev.message : null),
  }
  return rows.map((r, j) => (j === i ? merged : r))
}

export function percent(r: Pick<StepRow, 'done' | 'total'>): number | null {
  if (!r.total) return null
  return Math.min(100, Math.floor((r.done * 100) / r.total))
}
