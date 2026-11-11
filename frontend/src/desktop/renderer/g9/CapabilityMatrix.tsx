// Model yetenek matrisi (madde 50-ek): tüm sağlayıcıların modelleri tek tabloda; araç, görsel, akıl yürütme, bağlam, fiyat.
import { useId, useMemo, useState } from 'react'
import type { CatalogModel } from '../../../core/model-catalog.ts'
import { formatTokens, formatUsd } from '../g7-format.ts'
import { providerLabel, ProviderFilter, providerMatches, type ProviderFilterValue } from './ProviderBadge.tsx'
import { modelProvider } from '../../../core/model-registry-g9.ts'

const MAX_ROWS = 300
const yes = (v: boolean) => (v ? 'Var' : '—')

export function CapabilityMatrix({ models }: { models: readonly CatalogModel[] }) {
  const id = useId()
  const [provider, setProvider] = useState<ProviderFilterValue>('all')
  const [query, setQuery] = useState('')
  const rows = useMemo(() => {
    const q = query.trim().toLocaleLowerCase('tr')
    return models.filter((m) => providerMatches(m, provider) && (!q || `${m.name} ${m.id}`.toLocaleLowerCase('tr').includes(q)))
  }, [models, provider, query])

  if (models.length === 0) return <p className="g7-note">Model listesi henüz yüklenmedi.</p>
  return (
    <div className="g7-stack">
      <div className="g7-table-tools">
        <label htmlFor={`${id}-q`} className="sr-only">Modellerde ara</label>
        <input id={`${id}-q`} className="menu-search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Model ara..." />
        <ProviderFilter models={models} value={provider} onChange={setProvider} />
        <span className="g7-note">{rows.length} model</span>
      </div>
      <div className="g7-table-wrap">
        <table className="g7-table">
          <caption className="sr-only">Model yetenekleri ve fiyatları</caption>
          <thead>
            <tr>
              <th scope="col">Model</th>
              <th scope="col">Sağlayıcı</th>
              <th scope="col">Araç</th>
              <th scope="col">Görsel</th>
              <th scope="col">Akıl yürütme</th>
              <th scope="col" className="num">Bağlam</th>
              <th scope="col" className="num">Girdi $/1M</th>
              <th scope="col" className="num">Çıktı $/1M</th>
              <th scope="col" className="num">Önbellek $/1M</th>
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, MAX_ROWS).map((m) => (
              <tr key={m.id}>
                <th scope="row" className="g7-model-cell">
                  <span className="g7-row-title">{m.name}</span>
                  <span className="g7-mono g9-dim">{m.id}</span>
                </th>
                <td>{providerLabel(modelProvider(m))}</td>
                <td>{yes(m.supportsTools)}</td>
                <td>{yes(m.vision)}</td>
                <td>{yes(m.reasoning)}</td>
                <td className="num">{formatTokens(m.contextTokens)}</td>
                <td className="num">{formatUsd(m.price?.input ?? null)}</td>
                <td className="num">{formatUsd(m.price?.output ?? null)}</td>
                <td className="num">{formatUsd(m.price?.cacheInput ?? null)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {rows.length > MAX_ROWS && <p className="g7-note">İlk {MAX_ROWS} model gösteriliyor; aramayla daralt.</p>}
      <p className="g7-note">"—": desteklenmiyor veya sağlayıcı bildirmiyor. Fiyatı bilinmeyenler için Fiyat tablosu kullanılabilir.</p>
    </div>
  )
}
