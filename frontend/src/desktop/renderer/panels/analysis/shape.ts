// Analiz bulgularının süzülmesi ve sayımı (saf mantık; shape.test.ts).
import type { Finding, Severity } from '../../../ipc-analysis.ts'

export const SEVERITIES: readonly Severity[] = ['critical', 'high', 'medium', 'low', 'info']

export const SEVERITY_LABELS: Record<Severity, string> = {
  critical: 'Kritik',
  high: 'Yüksek',
  medium: 'Orta',
  low: 'Düşük',
  info: 'Bilgi',
}

export type Counts = Record<Severity, number>

export function counts(findings: readonly Finding[]): Counts {
  const c: Counts = { critical: 0, high: 0, medium: 0, low: 0, info: 0 }
  for (const f of findings) c[f.severity] += 1
  return c
}

/** Seçili önem düzeyleri ve metin araması (kural, dosya, mesaj, CWE). */
export function filterFindings(findings: readonly Finding[], shown: ReadonlySet<Severity>, query: string): Finding[] {
  const q = query.trim().toLowerCase()
  return findings.filter((f) => {
    if (!shown.has(f.severity)) return false
    if (!q) return true
    return [f.rule, f.file, f.message, f.cwe ?? '', f.source].some((s) => s.toLowerCase().includes(q))
  })
}

/** `dosya:satır` (sütun varsa `:sütun`) */
export function location(f: Finding): string {
  return `${f.file}:${f.line}${f.column ? `:${f.column}` : ''}`
}
