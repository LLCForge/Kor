// §28.3 kural bulgusu yardımcıları (RulesBadge ve kural paneli).
import type { RuleFinding } from '../../ipc.gen.ts'

export interface FindingCounts {
  violated: number
  relevant: number
  applied: number
}

export function countFindings(findings: readonly RuleFinding[]): FindingCounts {
  const n: FindingCounts = { violated: 0, relevant: 0, applied: 0 }
  for (const f of findings) n[f.verdict] += 1
  return n
}

/** `src/a.rs:12` (değişen dosyadaki yer) ya da boş. */
export function findingLocation(f: RuleFinding): string {
  if (!f.file) return ''
  return f.fileLine != null ? `${f.file}:${f.fileLine}` : f.file
}

/** İhlaller önce, sonra ilgili, sonra uyulan; aynı grupta kaynak sırası. */
export function sortFindings(findings: readonly RuleFinding[]): RuleFinding[] {
  const rank = { violated: 0, relevant: 1, applied: 2 } as const
  return [...findings].sort((a, b) => rank[a.verdict] - rank[b.verdict] || a.source.localeCompare(b.source) || a.line - b.line)
}
