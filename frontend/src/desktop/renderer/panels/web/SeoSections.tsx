// K6'nın SEO bölümleri (ADR 0055) web paneline buradan takılır.
import { registerSection } from './section-registry.ts'
import { seoSections } from './seo/index.tsx'

for (const s of seoSections) registerSection({ id: s.id, label: s.label, component: s.component })
