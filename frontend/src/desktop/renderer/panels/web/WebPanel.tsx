// Web siteleri görünümü (§29): bölümler sections.ts'ten gelir (K5 site bölümleri; K6 SEO bölümleri aynı
// kayıtla eklenir). Bölümler açılır/kapanır, açıklar hatırlanmaz (her açılışta özet görünür).
import { useId } from 'react'
import type { KorPanelProps } from '../registry.ts'
import { webSections } from './section-registry.ts'
import './sections.ts'

export function WebPanel(_: KorPanelProps) {
  const id = useId()
  return (
    <div className="ks kw">
      {webSections().map((s, i) => (
        <section key={s.id} className="kg-card" aria-labelledby={`${id}-${s.id}`}>
          <details open={i === 0}>
            <summary>
              <h3 id={`${id}-${s.id}`} className="kg-card-title ks-title kw-inline">{s.label}</h3>
            </summary>
            <div className="kw-body">
              <s.component />
            </div>
          </details>
        </section>
      ))}
    </div>
  )
}
