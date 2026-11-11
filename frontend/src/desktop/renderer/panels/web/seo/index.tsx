// SEO bölümleri (§29.5–29.10, §29.15, §29.16, ADR 0055): K5'in web paneline (`renderer/panels/web/`) bölüm
// dizisi olarak takılır. Her bölüm kendi kartıdır; ortak sınıflar graph/security/perf panellerinden.
import { BarChart3, Eye, FileText, PenLine, ScanSearch, Users, Wand2 } from 'lucide-react'
import type { ComponentType } from 'react'
import '../../graph/panel.css'
import '../../security/panel.css'
import '../../perf/panel.css'
import './seo.css'
import { AgentsSection } from './AgentsSection.tsx'
import { AuditSection } from './AuditSection.tsx'
import { CompetitorsSection } from './CompetitorsSection.tsx'
import { ContentSection } from './ContentSection.tsx'
import { GoogleSection } from './GoogleSection.tsx'
import { LogsSection } from './LogsSection.tsx'
import { MetaSection } from './MetaSection.tsx'

export interface SeoSection {
  id: string
  /** Türkçe kaynak metin */
  label: string
  icon: ComponentType<{ size?: number }>
  component: ComponentType
}

export const seoSections: readonly SeoSection[] = [
  { id: 'seo-audit', label: 'SEO denetimi', icon: ScanSearch, component: AuditSection },
  { id: 'seo-agents', label: 'SEO ajan takımı', icon: Users, component: AgentsSection },
  { id: 'seo-google', label: 'Search Console', icon: BarChart3, component: GoogleSection },
  { id: 'seo-content', label: 'İçerik hattı', icon: PenLine, component: ContentSection },
  { id: 'seo-meta', label: 'Toplu meta', icon: Wand2, component: MetaSection },
  { id: 'seo-logs', label: 'Log analizi', icon: FileText, component: LogsSection },
  { id: 'seo-competitors', label: 'Rakip izleme', icon: Eye, component: CompetitorsSection },
]

/** Bütün SEO bölümleri alt alta (web paneli yoksa tek başına da kullanılabilir). */
export function SeoSections() {
  return (
    <div className="kp kseo">
      {seoSections.map(({ id, component: C }) => <C key={id} />)}
    </div>
  )
}
