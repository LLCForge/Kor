// Kenar çubuğu sohbet listesi için saf yardımcılar (gruplama, kullanım/maliyet metni); test edilebilir.
import type { Section } from '../../core/sections.ts'
import type { SessionMeta } from '../ipc.ts'
import { uiLocale, agoIntl } from './ui-locale.ts'

export type ModelPricing = Readonly<Record<string, { input: number; output: number }>>

export interface SessionGroups {
  pinned: SessionMeta[]
  sections: { section: Section; sessions: SessionMeta[] }[]
  projects: [string, SessionMeta[]][]
  archived: SessionMeta[]
}

export const projectName = (root: string) => root.split(/[\\/]/).filter(Boolean).pop() ?? root

/**
 * Arşivlenenler yalnızca arşiv görünümünde; sabitlenmişler en üstte; sonra kullanıcı bölümleri (boşlar dahil),
 * sonra proje klasörüne göre gruplar. Silinmiş bir bölüme ait sohbet proje grubuna düşer.
 */
export function groupSessions(sessions: readonly SessionMeta[], sections: readonly Section[]): SessionGroups {
  const known = new Set(sections.map((s) => s.id))
  const bySection = new Map<string, SessionMeta[]>()
  const byProject = new Map<string, SessionMeta[]>()
  const pinned: SessionMeta[] = []
  const archived: SessionMeta[] = []
  for (const s of sessions) {
    if (s.archived) archived.push(s)
    else if (s.pinned) pinned.push(s)
    else if (s.sectionId && known.has(s.sectionId)) bySection.set(s.sectionId, [...(bySection.get(s.sectionId) ?? []), s])
    else {
      const name = projectName(s.root)
      byProject.set(name, [...(byProject.get(name) ?? []), s])
    }
  }
  return {
    pinned,
    sections: sections.map((section) => ({ section, sessions: bySection.get(section.id) ?? [] })),
    projects: [...byProject.entries()],
    archived,
  }
}

const trimZero = (s: string) => s.replace(/\.0$/, '')

export function formatTokens(n: number): string {
  if (n < 1000) return String(n)
  if (n < 999_950) return `${trimZero((n / 1000).toFixed(1))}k`
  return `${trimZero((n / 1_000_000).toFixed(1))}M`
}

/** Fiyatı bilinmeyen bir model varsa maliyet null: yanlış (eksik) toplam göstermektense hiç göstermemek. */
export function sessionCost(usage: SessionMeta['usage'], pricing: ModelPricing | undefined): { tokens: number; costUsd: number | null } | null {
  let tokens = 0
  let costUsd: number | null = 0
  for (const [model, u] of Object.entries(usage ?? {})) {
    tokens += u.promptTokens + u.completionTokens
    const price = pricing?.[model]
    costUsd = price && costUsd !== null ? costUsd + (u.promptTokens * price.input + u.completionTokens * price.output) / 1e6 : null
  }
  return tokens > 0 ? { tokens, costUsd } : null
}

function formatCost(usd: number): string {
  if (usd > 0 && usd < 0.0001) return '< $0,0001'
  const digits = usd < 1 ? 4 : 2
  return `$${usd.toLocaleString(uiLocale(), { minimumFractionDigits: digits, maximumFractionDigits: digits })}`
}

/** "≈ 12.4k token · $0,0031"; kullanım yoksa null. */
export function formatUsage(usage: SessionMeta['usage'], pricing: ModelPricing | undefined): string | null {
  const summary = sessionCost(usage, pricing)
  if (!summary) return null
  const tokens = `≈ ${formatTokens(summary.tokens)} token`
  return summary.costUsd === null ? tokens : `${tokens} · ${formatCost(summary.costUsd)}`
}

/** cdesktop kenar çubuğu zamanı: "az önce", "8dk önce", "3sa önce", "2g önce", "3ay önce". */
export function relativeTime(at: number, now: number = Date.now()): string {
  const s = Math.max(0, Math.round((now - at) / 1000))
  const intl = agoIntl(s)
  if (intl) return intl
  if (s < 45) return 'az önce'
  const m = Math.round(s / 60)
  if (m < 60) return `${m}dk önce`
  const h = Math.round(m / 60)
  if (h < 24) return `${h}sa önce`
  const d = Math.round(h / 24)
  if (d < 30) return `${d}g önce`
  const mo = Math.round(d / 30)
  if (mo < 12) return `${mo}ay önce`
  return `${Math.round(d / 365)}y önce`
}

/** Kenar çubuğu araması: başlık, özet ve proje adı; boş sorgu her şeyi döner. Türkçe küçük harf eşlemesi. */
export function filterSessions<T extends Pick<SessionMeta, 'title' | 'recap' | 'root'>>(sessions: readonly T[], query: string): T[] {
  const terms = query.toLocaleLowerCase('tr').split(/\s+/).filter(Boolean)
  if (terms.length === 0) return [...sessions]
  return sessions.filter((s) => {
    const hay = `${s.title} ${s.recap ?? ''} ${projectName(s.root)}`.toLocaleLowerCase('tr')
    return terms.every((t) => hay.includes(t))
  })
}
