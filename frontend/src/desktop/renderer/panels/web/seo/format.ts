// SEO bölümlerinin saf yardımcıları (node --test ile sınanır).
import type { SeoAgentEvent, SeoFinding, SeoSeverity } from '../../../../ipc-seo.ts'

export const SEVERITIES: readonly SeoSeverity[] = ['critical', 'high', 'medium', 'low', 'info']

export const SEVERITY_LABEL: Record<SeoSeverity, string> = {
  critical: 'Kritik',
  high: 'Yüksek',
  medium: 'Orta',
  low: 'Düşük',
  info: 'Bilgi',
}

/** Ajan kimliği → Türkçe ad (kor_web::seo::agents::AGENTS ile aynı). */
export const AGENT_LABEL: Record<string, string> = {
  crawler: 'Tarayıcı',
  technical: 'Teknik',
  performance: 'Performans',
  content: 'İçerik',
  structured: 'Yapılandırılmış veri',
  local: 'Yerel / Türkçe',
  security: 'Güvenlik & güven',
  competitor: 'Rakip',
  log: 'Log',
  fixer: 'Düzeltici',
  reporter: 'Raporcu',
}

export function counts(findings: readonly SeoFinding[]): Record<SeoSeverity, number> {
  const c: Record<SeoSeverity, number> = { critical: 0, high: 0, medium: 0, low: 0, info: 0 }
  for (const f of findings) c[f.severity] += 1
  return c
}

/** `url:satır` (satır 1 ise yalnız URL: sayfa düzeyi bulgu). */
export function location(f: Pick<SeoFinding, 'file' | 'line'>): string {
  return f.line > 1 ? `${f.file}:${f.line}` : f.file
}

export function filterFindings(findings: readonly SeoFinding[], min: SeoSeverity, text: string): SeoFinding[] {
  const limit = SEVERITIES.indexOf(min)
  const q = text.trim().toLowerCase()
  return findings.filter(
    (f) => SEVERITIES.indexOf(f.severity) <= limit && (!q || f.file.toLowerCase().includes(q) || f.rule.toLowerCase().includes(q) || f.message.toLowerCase().includes(q)),
  )
}

/** Satır başına bir URL; boşluklar ve yinelenenler atılır, yalnız http(s). */
export function parseUrls(text: string): string[] {
  const out: string[] = []
  for (const raw of text.split(/[\n,\s]+/)) {
    const u = raw.trim()
    if (/^https?:\/\/\S+$/i.test(u) && !out.includes(u)) out.push(u)
  }
  return out
}

export interface AgentCard {
  id: string
  state: 'running' | 'done' | 'skipped'
  findings: number
  detail: string
}

/** Ajan olaylarını kart durumuna katlar (sıra korunur). */
export function foldAgent(cards: readonly AgentCard[], e: SeoAgentEvent): AgentCard[] {
  const next = cards.map((c) => ({ ...c }))
  let card = next.find((c) => c.id === e.agent)
  if (!card) {
    card = { id: e.agent, state: 'running', findings: 0, detail: '' }
    next.push(card)
  }
  switch (e.state) {
    case 'started':
      card.state = 'running'
      break
    case 'progress':
      card.detail = e.detail
      break
    case 'done':
      card.state = 'done'
      card.findings = e.findings
      card.detail = ''
      break
    case 'skipped':
      card.state = 'skipped'
      card.detail = e.reason
      break
  }
  return next
}

export function pct(v: number): string {
  return `${(v * 100).toFixed(1)}%`
}

export function usd(v: number | null): string {
  return v === null ? 'bilinmiyor' : `≈ $${v.toFixed(4)}`
}
