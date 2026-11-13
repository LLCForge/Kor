// Model kataloğu (§4.4): Artificial Analysis zekâ/kodlama skorları, fiyat, bağlam, yetenekler; arama, sağlayıcıya
// göre gruplama, yenileme ve kaynak durumu. Veri `catalog.get` (kor catalog ile aynı yerel kopya); AA skorları
// yalnızca merkezi katalogdan gelir, kullanıcı anahtarı yok (ADR-0003).
import { Eye, RefreshCw, Search, Wrench } from 'lucide-react'
import { useDeferredValue, useId, useMemo, useState } from 'react'
import useSWR from 'swr'
import type { CatalogEntry } from '../../../ipc-providers.ts'
import { errorMessage } from '../../api.ts'
import { formatTokens, formatUsd } from '../../g7-format.ts'
import { AaAttribution, ScoreBar } from '../../ModelDetail.tsx'
import { kor } from './bridge.ts'
import { ago, groupCatalog, maxOf, SORT_LABELS, type SortKey } from './catalog-view.ts'
import { ProviderLogo } from './ProviderLogo.tsx'
import { uiLocale } from '../../ui-locale.ts'

const PAGE = 30
const ORIGIN_LABELS = { published: 'Kor model kataloğu', openrouter: 'OpenRouter (yedek kaynak)' } as const

function price(n: number | null): string {
  return n === null ? '—' : formatUsd(n, 2)
}

function Row({ m, maxI, maxC }: { m: CatalogEntry; maxI: number; maxC: number }) {
  return (
    <li className="kp-model">
      <span className="kp-name" translate="no" title={m.model_id}>
        {m.model_id}
      </span>
      <span className="kp-scores">
        <ScoreBar label="Zekâ" value={m.aa_intelligence_index} max={maxI} />
        <ScoreBar label="Kodlama" value={m.aa_coding_index} max={maxC} />
      </span>
      <span className="kp-meta">
        <span title="Giriş / çıkış fiyatı (1M token başına)">
          {price(m.input_price_per_m)} / {price(m.output_price_per_m)}
        </span>
        <span title="Bağlam penceresi">{formatTokens(m.context)}</span>
        {m.tool_calling && (
          <span title="Araç çağırma">
            <Wrench size={12} aria-hidden="true" />
            <span className="sr-only">Araç çağırma</span>
          </span>
        )}
        {m.vision && (
          <span title="Görsel girdi">
            <Eye size={12} aria-hidden="true" />
            <span className="sr-only">Görsel girdi</span>
          </span>
        )}
        {m.lmarena_rating !== null && (
          <span title="LMArena puanı (lmarena.ai)">
            <span translate="no">Arena {Math.round(m.lmarena_rating)}</span>
          </span>
        )}
        {m.livebench_avg !== null && (
          <span title="LiveBench ortalaması (livebench.ai)">
            <span translate="no">LiveBench {m.livebench_avg.toFixed(1)}</span>
          </span>
        )}
        {m.license && (
          <span className="badge neutral" translate="no">
            {m.license}
          </span>
        )}
      </span>
    </li>
  )
}

export function CatalogTab() {
  const searchId = useId()
  const sortId = useId()
  const { data, error, isLoading, mutate } = useSWR('catalog.get', () => kor.catalog.get(false), { revalidateOnFocus: false })
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [sort, setSort] = useState<SortKey>('intelligence')
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set())
  const [shown, setShown] = useState<Record<string, number>>({})
  const deferred = useDeferredValue(query)

  const models = data?.models
  const groups = useMemo(() => groupCatalog(models ?? [], deferred, sort), [models, deferred, sort])
  const maxI = useMemo(() => maxOf(models ?? [], (m) => m.aa_intelligence_index), [models])
  const maxC = useMemo(() => maxOf(models ?? [], (m) => m.aa_coding_index), [models])
  const total = groups.reduce((n, g) => n + g.models.length, 0)

  const refresh = async () => {
    setRefreshing(true)
    setRefreshError(null)
    try {
      await mutate(kor.catalog.get(true), { revalidate: false })
    } catch (e) {
      setRefreshError(errorMessage(e))
    } finally {
      setRefreshing(false)
    }
  }

  const toggle = (p: string, isOpen: boolean) =>
    setOpen((prev) => {
      const next = new Set(prev)
      if (isOpen) next.add(p)
      else next.delete(p)
      return next
    })

  return (
    <section className="kp-section" aria-labelledby="kp-cat-h">
      <div className="kp-head">
        <h3 id="kp-cat-h" className="kp-h">
          Model kataloğu
        </h3>
        <button type="button" className="btn small" data-kor-cmd="catalog.get" onClick={() => void refresh()} disabled={refreshing}>
          <RefreshCw size={14} aria-hidden="true" className={refreshing ? 'g7-spin' : undefined} /> {refreshing ? 'Yenileniyor...' : 'Yenile'}
        </button>
      </div>
      {data && (
        <p className="g7-note kp-parts" role="status">
          <span>{ORIGIN_LABELS[data.origin]}</span>
          <span>{`${data.models.length.toLocaleString(uiLocale())} model`}</span>
          <span title="Katalogun alındığı zaman">{ago(data.fetchedAt)}</span>
          {data.fromCache && <span>önbellekten</span>}
        </p>
      )}
      {data?.notices.map((n) => (
        <p key={n} className="g9-warn">
          {n}
        </p>
      ))}
      {(error || refreshError) && (
        <p className="error-text" role="alert">
          {refreshError ?? errorMessage(error)}
        </p>
      )}
      {isLoading && <p className="muted">Katalog yükleniyor...</p>}
      {data && data.models.length === 0 && <p className="kp-empty">Katalog boş. İnternet varsa Yenile ile yeniden dene; gizlilik modunda yalnızca önbellek kullanılır.</p>}
      {data && data.models.length > 0 && (
        <>
          <div className="kp-toolbar">
            <label htmlFor={searchId} className="sr-only">
              Model ara
            </label>
            <span className="kp-search">
              <Search size={14} aria-hidden="true" />
              <input id={searchId} className="text-input" type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Model, sağlayıcı veya lisans ara" maxLength={200} />
            </span>
            <label htmlFor={sortId} className="sr-only">
              Sıralama
            </label>
            <select id={sortId} className="g7-select" value={sort} onChange={(e) => setSort(e.target.value as SortKey)}>
              {(Object.keys(SORT_LABELS) as SortKey[]).map((k) => (
                <option key={k} value={k}>
                  {SORT_LABELS[k]}
                </option>
              ))}
            </select>
          </div>
          {total === 0 && <p className="kp-empty">Aramayla eşleşen model yok.</p>}
          <div className="kp-groups">
            {groups.map((g) => {
              const isOpen = deferred.trim() !== '' || groups.length === 1 || open.has(g.provider)
              const limit = shown[g.provider] ?? PAGE
              return (
                <details key={g.provider} className="kp-group" open={isOpen} onToggle={(e) => toggle(g.provider, e.currentTarget.open)}>
                  <summary className="kp-group-head">
                    <ProviderLogo provider={g.provider} modelId={g.models[0]?.model_id} />
                    <span translate="no">{g.provider}</span>
                    <span className="g7-tab-count">{g.models.length}</span>
                  </summary>
                  {isOpen && (
                    <ul className="kp-models">
                      {g.models.slice(0, limit).map((m) => (
                        <Row key={m.model_id} m={m} maxI={maxI} maxC={maxC} />
                      ))}
                    </ul>
                  )}
                  {isOpen && g.models.length > limit && (
                    <button type="button" className="btn ghost small" onClick={() => setShown((s) => ({ ...s, [g.provider]: g.models.length }))}>
                      {`Tümünü göster (${g.models.length})`}
                    </button>
                  )}
                </details>
              )
            })}
          </div>
          <AaAttribution />
        </>
      )}
    </section>
  )
}
