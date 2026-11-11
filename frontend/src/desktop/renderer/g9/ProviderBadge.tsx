// Model seçicide sağlayıcı rozeti ve sağlayıcı filtresi (g7 seçicisine [g9] satırlarıyla bağlanır).
import { useId, useMemo } from 'react'
import type { CatalogModel } from '../../../core/model-catalog.ts'
import { modelProvider } from '../../../core/model-registry-g9.ts'
import { getProvider, type ProviderId } from '../../../core/providers.ts'
import { brandIconFor } from './BrandIcons.tsx'

export type ProviderFilterValue = ProviderId | 'all'

export function providerLabel(id: ProviderId): string {
  return getProvider(id)?.label ?? id
}

export function providerMatches(m: CatalogModel, value: ProviderFilterValue): boolean {
  return value === 'all' || modelProvider(m) === value
}

/** Venice modelleri: sağlayıcı metni yok ama model marka ikonu (deepseek/qwen...) gösterilir;
 *  diğer sağlayıcılar ikon + adla işaretlenir. */
export function ProviderBadge({ model }: { model: CatalogModel }) {
  const id = modelProvider(model)
  const icon = brandIconFor(model)
  if (id === 'venice') {
    if (!icon) return null
    return (
      <span className="badge neutral g9-provider-badge g9-brand-only" title={model.name}>
        {icon}
      </span>
    )
  }
  return (
    <span className="badge neutral g9-provider-badge" title={`Sağlayıcı: ${providerLabel(id)}`}>
      {icon}
      {providerLabel(id)}
    </span>
  )
}

/** Listede birden fazla sağlayıcı varsa görünür. */
export function ProviderFilter({ models, value, onChange }: { models: readonly CatalogModel[]; value: ProviderFilterValue; onChange: (v: ProviderFilterValue) => void }) {
  const id = useId()
  const counts = useMemo(() => {
    const map = new Map<ProviderId, number>()
    for (const m of models) map.set(modelProvider(m), (map.get(modelProvider(m)) ?? 0) + 1)
    return map
  }, [models])
  if (counts.size < 2 && value === 'all') return null
  return (
    <>
      <label htmlFor={id} className="sr-only">Sağlayıcı</label>
      <select id={id} className="g7-select g9-provider-filter" value={value} onChange={(e) => onChange(e.target.value as ProviderFilterValue)}>
        <option value="all">Tüm sağlayıcılar</option>
        {[...counts.entries()].map(([p, n]) => (
          <option key={p} value={p}>
            {providerLabel(p)} ({n})
          </option>
        ))}
      </select>
    </>
  )
}
