// Web siteleri paneli bölümleri. Her bölüm kendi dosyasında `registerSection(...)` çağırır ve sections.ts'e
// tek bir `import './X.tsx'` satırıyla eklenir (K5: sunucular, WordPress, yönlendirmeler, teklif, müşteriler,
// Türkiye; K6: SEO bölümleri). Bu dosya hiçbir bölümü içe aktarmaz: döngüsel içe aktarma olmaz.
import type { ComponentType } from 'react'

export interface WebSection {
  /** Benzersiz kimlik, ör. `hosts` */
  id: string
  /** Türkçe başlık */
  label: string
  component: ComponentType
}

const sections: WebSection[] = []

export function registerSection(s: WebSection): void {
  if (!sections.some((x) => x.id === s.id)) sections.push(s)
}

export function webSections(): readonly WebSection[] {
  return sections
}
