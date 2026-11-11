// Evrensel Git (§8.8, ADR 0040) köprü tipleri. Rust: src-tauri/src/ns/forge.rs.
// ipc.ts bu ad alanını tiplemiyor; erişim renderer/forge/bridge.ts içinde tek yerde.
// Types are GENERATED from backend/crates/kor-core/schema/ipc.json (spec §16.3, ADR 0061); edit the schema, not this file.
import type { ForgeKind, ForgeRemote, ForgeDetect, PrInfo, PrRef, PrDescription, Severity__forge as Severity, Finding__forge as Finding, ReviewReport, ReviewResult, PersonaInfo, PrComment, RespondDraft, ReviewOptions, ForgeApi } from './ipc.gen.ts'
export type { ForgeKind, ForgeRemote, ForgeDetect, PrInfo, PrRef, PrDescription, Severity, Finding, ReviewReport, ReviewResult, PersonaInfo, PrComment, RespondDraft, ReviewOptions, ForgeApi }

const ORDER: Severity[] = ['critical', 'high', 'medium', 'low', 'info']

/** Bulguları önem sırasına göre sayar (özet satırı için). */
export function severityCounts(findings: Finding[]): [Severity, number][] {
  return ORDER.map((s) => [s, findings.filter((f) => f.severity === s).length] as [Severity, number]).filter(([, n]) => n > 0)
}

/** `dosya:satır` ya da yalnızca dosya. */
export const findingLocation = (f: Finding): string => (f.line == null ? f.file : `${f.file}:${f.line}`)
