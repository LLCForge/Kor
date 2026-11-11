// Analiz paneli bölümleri. Her bölüm kendi dosyasında `registerSection(...)` çağırır ve sections.ts'e
// tek bir `import './X.tsx'` satırıyla eklenir (J2 lisanslar, J3 kalite/borç…). Bu dosya hiçbir bölümü
// içe aktarmaz: döngüsel içe aktarma olmaz.
import type { ComponentType } from 'react'

export interface AnalysisSection {
  /** Benzersiz kimlik, ör. `security` */
  id: string
  /** Türkçe başlık */
  label: string
  component: ComponentType
}

const sections: AnalysisSection[] = []

export function registerSection(s: AnalysisSection): void {
  if (!sections.some((x) => x.id === s.id)) sections.push(s)
}

export function analysisSections(): readonly AnalysisSection[] {
  return sections
}
