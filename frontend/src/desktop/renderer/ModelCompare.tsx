import { ArrowDown, ArrowUp, FlaskConical, RefreshCw, X } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import { mutate } from 'swr'
import {
  blendedPrice,
  CATALOG_TABS,
  codingRanks,
  defaultDir,
  filterCatalog,
  recommendForCoding,
  sortCatalog,
  type CatalogModel,
  type CatalogTab,
  type SortDir,
  type SortKey,
} from '../../core/model-catalog.ts'
import { api, errorMessage } from './api.ts'
import { CATALOG_KEY, openG7Dialog, useCatalog } from './g7-bus.ts'
import { formatScore, formatTokens, formatUsd, PRIVACY_LABELS } from './g7-format.ts'
import { G7Modal } from './G7Modal.tsx'
import { AaAttribution, maxScoresOf, ModelBadges, ScoreBar } from './ModelDetail.tsx'
import { uiLocale } from './ui-locale.ts'

const MAX_SELECTED = 3

interface Column {
  key: SortKey
  label: string
  title: string
}

const COLUMNS: readonly Column[] = [
  { key: 'coding', label: 'Kodlama', title: 'Artificial Analysis kodlama indeksi' },
  { key: 'intelligence', label: 'Zeka', title: 'Artificial Analysis zeka indeksi' },
  { key: 'math', label: 'Matematik', title: 'Artificial Analysis matematik indeksi' },
  { key: 'speed', label: 'Hız', title: 'Saniyede çıktı token (Artificial Analysis, medyan)' },
  { key: 'inputPrice', label: 'Girdi', title: 'USD / 1M girdi token' },
  { key: 'outputPrice', label: 'Çıktı', title: 'USD / 1M çıktı token' },
  { key: 'cachePrice', label: 'Önbellek', title: 'USD / 1M önbellekten okunan girdi token' },
  { key: 'context', label: 'Bağlam', title: 'Bağlam penceresi (token)' },
]

/** Karşılaştırma kartlarında her ölçüt için en iyi değer vurgulanır. */
interface Metric {
  label: string
  value: (m: CatalogModel) => number | null
  format: (n: number | null) => string
  better: 'high' | 'low'
}

const METRICS: readonly Metric[] = [
  { label: 'Kodlama', value: (m) => m.aa?.coding ?? null, format: formatScore, better: 'high' },
  { label: 'Zeka', value: (m) => m.aa?.intelligence ?? null, format: formatScore, better: 'high' },
  { label: 'Matematik', value: (m) => m.aa?.math ?? null, format: formatScore, better: 'high' },
  { label: 'Hız (token/sn)', value: (m) => m.aa?.tokensPerSecond ?? null, format: formatScore, better: 'high' },
  { label: 'İlk token (sn)', value: (m) => m.aa?.ttftSeconds ?? null, format: formatScore, better: 'low' },
  { label: 'Girdi (1M)', value: (m) => m.price?.input ?? null, format: (n) => formatUsd(n), better: 'low' },
  { label: 'Çıktı (1M)', value: (m) => m.price?.output ?? null, format: (n) => formatUsd(n), better: 'low' },
  { label: 'Önbellek okuma (1M)', value: (m) => m.price?.cacheInput ?? null, format: (n) => formatUsd(n), better: 'low' },
  { label: 'Karışık fiyat (3:1)', value: (m) => blendedPrice(m.price), format: (n) => formatUsd(n), better: 'low' },
  { label: 'Bağlam', value: (m) => m.contextTokens, format: formatTokens, better: 'high' },
  { label: 'En fazla çıktı', value: (m) => m.maxCompletionTokens, format: formatTokens, better: 'high' },
]

function capsShort(m: CatalogModel): string {
  return [m.supportsTools && 'Araç', m.vision && 'Görsel', m.reasoning && 'Akıl', m.codeOptimized && 'Kod', m.webSearch && 'Web'].filter(Boolean).join(' · ') || '—'
}

interface ModelCompareProps {
  open: boolean
  onClose: () => void
  currentModel: string
  onUseModel: (id: string) => void
}

export function ModelCompare({ open, onClose, currentModel, onUseModel }: ModelCompareProps) {
  const titleId = useId()
  const { data, error, isLoading } = useCatalog(open)
  const [tab, setTab] = useState<CatalogTab>('all')
  const [query, setQuery] = useState('')
  const [sort, setSort] = useState<{ key: SortKey; dir: SortDir }>({ key: 'coding', dir: 'desc' })
  const [selected, setSelected] = useState<string[]>([])
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState<string | null>(null)

  const models = useMemo(() => data?.models ?? [], [data])
  const aliases = useMemo(() => data?.aliases ?? {}, [data])
  const ranks = useMemo(() => codingRanks(models), [models])
  const maxScores = useMemo(() => maxScoresOf(models), [models])
  const recs = useMemo(() => recommendForCoding(models), [models])
  const rows = useMemo(() => sortCatalog(filterCatalog(models, { tab, query, aliases }), sort.key, sort.dir), [models, tab, query, aliases, sort])
  const chosen = selected.map((id) => models.find((m) => m.id === id)).filter((m): m is CatalogModel => m !== undefined)

  const toggle = (id: string) =>
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : prev.length < MAX_SELECTED ? [...prev, id] : prev))

  const sortBy = (key: SortKey) =>
    setSort((prev) => (prev.key === key ? { key, dir: prev.dir === 'asc' ? 'desc' : 'asc' } : { key, dir: defaultDir(key) }))

  const use = (id: string) => {
    onUseModel(id)
    onClose()
  }

  const refresh = async () => {
    setRefreshing(true)
    setRefreshError(null)
    try {
      await mutate(CATALOG_KEY, api.g7GetCatalog(true), { revalidate: false })
    } catch (err) {
      console.error('Model kataloğu yenilenemedi:', err)
      setRefreshError(`Yenilenemedi: ${errorMessage(err)}`)
    } finally {
      setRefreshing(false)
    }
  }

  const ariaSort = (key: SortKey) => (sort.key === key ? (sort.dir === 'asc' ? 'ascending' : 'descending') : 'none')

  return (
    <G7Modal
      open={open}
      onClose={onClose}
      titleId={titleId}
      title="Modelleri karşılaştır"
      className="g7-full"
      actions={
        <button type="button" className="action-btn" onClick={() => void refresh()} disabled={refreshing} aria-label="Verileri yenile" title="Yenile">
          <RefreshCw size={16} aria-hidden="true" className={refreshing ? 'g7-spin' : undefined} />
        </button>
      }
    >
      <div className="g7-full-body">
        {(error || refreshError) && <p className="error-box" role="alert">{error ? errorMessage(error) : refreshError}</p>}
        {isLoading && !data && <p className="muted">Modeller ve puanlar yükleniyor...</p>}

        {data && (
          <>
            {data.aa.message && <p className="g7-note">Artificial Analysis: {data.aa.message}</p>}

            <section className="g7-section" aria-labelledby={`${titleId}-rec`}>
              <h3 id={`${titleId}-rec`} className="g7-h3">Kodlama için önerilen</h3>
              {recs.fromVenice && (
                <p className="g7-note">Puan verisi olmadığı için katalog işaretlerine (kodlama varsayılanı, en zeki, koda optimize) göre seçildi.</p>
              )}
              <div className="g7-cards">
                {recs.best.map((m, i) => (
                  <RecCard key={m.id} label={recs.fromVenice ? 'Önerilen' : `Kodlamada #${i + 1}`} model={m} current={currentModel} onUse={use} maxCoding={maxScores.coding} />
                ))}
                {recs.value && <RecCard label="Fiyat / performans" model={recs.value} current={currentModel} onUse={use} maxCoding={maxScores.coding} />}
                {recs.uncensored && <RecCard label="En iyi sansürsüz" model={recs.uncensored} current={currentModel} onUse={use} maxCoding={maxScores.coding} />}
                {recs.best.length === 0 && <p className="muted">Öneri için yeterli veri yok.</p>}
              </div>
            </section>

            {chosen.length > 0 && (
              <section className="g7-section" aria-labelledby={`${titleId}-side`}>
                <div className="g7-section-head">
                  <h3 id={`${titleId}-side`} className="g7-h3">Yan yana ({chosen.length}/{MAX_SELECTED})</h3>
                  {chosen.length >= 2 && (
                    <button type="button" className="btn small" onClick={() => openG7Dialog({ kind: 'probe', models: chosen.map((m) => m.id) })}>
                      <FlaskConical size={14} aria-hidden="true" /> Aynı istemle test et
                    </button>
                  )}
                </div>
                {chosen.length < 2 && <p className="g7-note">Karşılaştırmak için tablodan en az bir model daha seç.</p>}
                <div className="g7-side">
                  {chosen.map((m) => (
                    <article key={m.id} className="g7-side-card" aria-label={m.name}>
                      <header className="g7-side-head">
                        <div className="g7-side-name">
                          <strong>{m.name}</strong>
                          <span className="menu-desc">{m.id}</span>
                        </div>
                        <button type="button" className="action-btn" onClick={() => toggle(m.id)} aria-label={`${m.name} karşılaştırmadan çıkar`}>
                          <X size={14} aria-hidden="true" />
                        </button>
                      </header>
                      <div className="g7-side-badges">
                        <ModelBadges model={m} rank={ranks.get(m.id)} />
                      </div>
                      <dl className="g7-side-metrics">
                        {METRICS.map((metric) => {
                          const v = metric.value(m)
                          const others = chosen.map(metric.value).filter((x): x is number => x !== null)
                          const best = metric.better === 'high' ? Math.max(...others) : Math.min(...others)
                          const isBest = v !== null && chosen.length >= 2 && others.length >= 2 && v === best
                          return (
                            <div key={metric.label} className={isBest ? 'best' : undefined}>
                              <dt>{metric.label}</dt>
                              <dd>
                                {metric.format(v)}
                                {isBest && <span className="sr-only"> (en iyi)</span>}
                              </dd>
                            </div>
                          )
                        })}
                        <div>
                          <dt>Gizlilik</dt>
                          <dd>{m.privacy ? PRIVACY_LABELS[m.privacy].label : '—'}</dd>
                        </div>
                      </dl>
                      <button type="button" className="btn small primary" onClick={() => use(m.id)} disabled={m.id === currentModel}>
                        {m.id === currentModel ? 'Kullanılıyor' : 'Bu modeli kullan'}
                      </button>
                    </article>
                  ))}
                </div>
              </section>
            )}

            <section className="g7-section" aria-labelledby={`${titleId}-all`}>
              <div className="g7-section-head">
                <h3 id={`${titleId}-all`} className="g7-h3">Tüm modeller</h3>
                <span className="g7-note">Karşılaştırmak için en fazla {MAX_SELECTED} model seç. Sansürsüz modeller her sıralamada üstte kalır.</span>
              </div>
              <div className="g7-table-tools">
                <label htmlFor={`${titleId}-q`} className="sr-only">Model ara</label>
                <input
                  id={`${titleId}-q`}
                  className="menu-search g7-grow"
                  placeholder="Model veya takma ad ara..."
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
                <div className="g7-tabs" role="tablist" aria-label="Model filtresi">
                  {CATALOG_TABS.map((t) => (
                    <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} className="g7-tab" onClick={() => setTab(t.id)}>
                      {t.label}
                    </button>
                  ))}
                </div>
              </div>
              <div className="g7-table-wrap">
                <table className="g7-table">
                  <caption className="sr-only">Modeller: fiyat, bağlam, yetenekler ve Artificial Analysis puanları</caption>
                  <thead>
                    <tr>
                      <th scope="col"><span className="sr-only">Seç</span></th>
                      <th scope="col" aria-sort={ariaSort('name')}>
                        <SortButton label="Model" active={sort.key === 'name'} dir={sort.dir} onClick={() => sortBy('name')} />
                      </th>
                      {COLUMNS.map((c) => (
                        <th key={c.key} scope="col" aria-sort={ariaSort(c.key)} title={c.title} className="num">
                          <SortButton label={c.label} active={sort.key === c.key} dir={sort.dir} onClick={() => sortBy(c.key)} />
                        </th>
                      ))}
                      <th scope="col">Yetenekler</th>
                      <th scope="col">Gizlilik</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((m) => {
                      const isSelected = selected.includes(m.id)
                      return (
                        <tr key={m.id} className={isSelected ? 'selected' : undefined}>
                          <td>
                            <input
                              type="checkbox"
                              checked={isSelected}
                              disabled={!isSelected && selected.length >= MAX_SELECTED}
                              onChange={() => toggle(m.id)}
                              aria-label={`${m.name} karşılaştırmaya ekle`}
                            />
                          </td>
                          <th scope="row" className="g7-model-cell">
                            <span className="g7-row-title">
                              <span>{m.name}</span>
                              {m.uncensored && <span className="badge">Sansürsüz</span>}
                              {(ranks.get(m.id) ?? 99) <= 3 && <span className="badge g7-rank">#{ranks.get(m.id)}</span>}
                              {m.id === currentModel && <span className="badge neutral">Seçili</span>}
                            </span>
                            <span className="menu-desc">{m.id}</span>
                          </th>
                          <td className="num">
                            <ScoreBar label="Kodlama" value={m.aa?.coding} max={maxScores.coding} hideLabel />
                            {m.aa?.coding == null && '—'}
                          </td>
                          <td className="num">{formatScore(m.aa?.intelligence)}</td>
                          <td className="num">{formatScore(m.aa?.math)}</td>
                          <td className="num">{formatScore(m.aa?.tokensPerSecond)}</td>
                          <td className="num">{formatUsd(m.price?.input ?? null)}</td>
                          <td className="num">{formatUsd(m.price?.output ?? null)}</td>
                          <td className="num">{formatUsd(m.price?.cacheInput ?? null)}</td>
                          <td className="num">{formatTokens(m.contextTokens)}</td>
                          <td className="g7-nowrap">{capsShort(m)}</td>
                          <td title={m.privacy ? PRIVACY_LABELS[m.privacy].desc : undefined}>{m.privacy ? PRIVACY_LABELS[m.privacy].label : '—'}</td>
                        </tr>
                      )
                    })}
                  </tbody>
                </table>
                {rows.length === 0 && <p className="muted g7-empty">Bu filtreyle model yok.</p>}
              </div>
            </section>

            <p className="g7-note">
              Fiyatlar, bağlam ve yetenekler sohbet kataloğundan; kodlama/zeka/matematik indeksleri ve hız <AaAttribution compact /> verisidir
              {data.aa.fetchedAt ? ` (güncelleme: ${new Date(data.aa.fetchedAt).toLocaleString(uiLocale())})` : ''}. Eşleştirme model adlarına göre yapılır;
              "yaklaşık" işaretli eşleşmeler farklı bir sürüme ait olabilir.
            </p>
          </>
        )}
      </div>
    </G7Modal>
  )
}

function SortButton({ label, active, dir, onClick }: { label: string; active: boolean; dir: SortDir; onClick: () => void }) {
  return (
    <button type="button" className={`g7-sort${active ? ' active' : ''}`} onClick={onClick}>
      {label}
      {active && (dir === 'asc' ? <ArrowUp size={12} aria-hidden="true" /> : <ArrowDown size={12} aria-hidden="true" />)}
    </button>
  )
}

function RecCard(props: { label: string; model: CatalogModel; current: string; onUse: (id: string) => void; maxCoding: number }) {
  const { model: m } = props
  return (
    <article className="g7-rec" aria-label={`${props.label}: ${m.name}`}>
      <span className="g7-rec-label">{props.label}</span>
      <strong className="g7-rec-name">{m.name}</strong>
      <span className="menu-desc">{m.id}</span>
      <ScoreBar label="Kodlama" value={m.aa?.coding} max={props.maxCoding} />
      <span className="g7-rec-meta">
        {formatUsd(m.price?.input ?? null)} / {formatUsd(m.price?.output ?? null)} · {formatTokens(m.contextTokens)}
        {m.uncensored ? ' · Sansürsüz' : ''}
      </span>
      <button type="button" className="btn small" onClick={() => props.onUse(m.id)} disabled={m.id === props.current}>
        {m.id === props.current ? 'Kullanılıyor' : 'Kullan'}
      </button>
    </article>
  )
}
