import { Check, ChevronDown, Columns3, FlaskConical, RefreshCw, Star } from 'lucide-react'
import { useId, useLayoutEffect, useMemo, useState, type CSSProperties, type KeyboardEvent } from 'react'
import { mutate } from 'swr'
import {
  aliasesFor,
  CATALOG_TABS,
  codingRanks,
  filterCatalog,
  resolveModelAlias,
  SORT_KEYS,
  sortCatalog,
  type CatalogModel,
  type CatalogTab,
  type SortKey,
} from '../../core/model-catalog.ts'
import type { ModelInfo } from '../ipc.ts'
import { api, errorMessage } from './api.ts'
import { CATALOG_KEY, openG7Dialog, useCatalog } from './g7-bus.ts'
import { formatTokens, formatUsd } from './g7-format.ts'
import { AaAttribution, maxScoresOf, ModelBadges, ModelDetail, ScoreBar } from './ModelDetail.tsx'
import { usePopover } from './use-popover.ts'
import { ProviderFilter, providerLabel, providerMatches, type ProviderFilterValue } from './g9/ProviderBadge.tsx' // [g9]
import { modelProvider } from '../../core/model-registry-g9.ts'
import { useModelPrefs, sectionModels } from './model-prefs.ts'
import { ProviderLogo } from './panels/providers/ProviderLogo.tsx'
import { modelName } from './message-model.ts'
import { isDimmed, PrivacyBadge, usePrivacyCards } from './privacy/PrivacyBadge.tsx' // §31.2

interface ModelPickerProps {
  value: string
  models: ModelInfo[] | undefined
  error: string | null
  onChange: (id: string) => void
}

/** Katalog alınamazsa eski model listesinden asgari kayıt; seçici yine çalışır. */
function fromModelInfo(m: ModelInfo): CatalogModel {
  return {
    id: m.id,
    name: m.name,
    description: '',
    uncensored: m.uncensored,
    supportsTools: m.supportsTools,
    vision: false,
    reasoning: false,
    codeOptimized: false,
    webSearch: false,
    contextTokens: null,
    maxCompletionTokens: null,
    privacy: null,
    traits: [],
    beta: false,
    deprecatedAt: null,
    price: null,
    aa: null,
  }
}

// Seçicide yalnızca anlamlı sıralamalar; tüm sütunlar karşılaştırma penceresinde.
const PICKER_SORTS: readonly SortKey[] = ['recommended', 'coding', 'intelligence', 'price', 'context', 'speed']

export function ModelPicker({ value, models, error, onChange }: ModelPickerProps) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  // Mesaj kutusu kaydırılabilen sohbet alanının içinde: mutlak konumlu menü orada kırpılıyordu.
  // Menü sabit konumlanır ve tetikleyicinin ekrandaki yerine göre pencere içine sığdırılır.
  const [popStyle, setPopStyle] = useState<CSSProperties | undefined>(undefined)
  useLayoutEffect(() => {
    if (!open) return
    const place = () => {
      const rect = ref.current?.getBoundingClientRect()
      if (!rect) return
      const width = Math.min(620, window.innerWidth - 32)
      const left = Math.min(Math.max(16, rect.right - width), window.innerWidth - width - 16)
      // Yukarıda yeterli yer varsa yukarı, yoksa daha geniş tarafa açılır; yükseklik o boşluğa sığdırılır (6 px aralık + 16 px kenar payı).
      // Özel başlık çubuğu pencere kontrollerini taşır; menü onun altında kalır.
      const top = document.querySelector('.titlebar')?.getBoundingClientRect().bottom ?? 0
      const above = rect.top - top - 22
      const below = window.innerHeight - rect.bottom - 22
      const up = above >= 420 || above >= below
      const vertical: CSSProperties = up ? { bottom: window.innerHeight - rect.top + 6, top: 'auto' } : { top: rect.bottom + 6, bottom: 'auto' }
      setPopStyle({ position: 'fixed', left, right: 'auto', width, maxHeight: Math.min(600, Math.max(160, up ? above : below)), ...vertical })
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  }, [open, ref])
  const [query, setQuery] = useState('')
  const [tab, setTab] = useState<CatalogTab>('all')
  const [sort, setSort] = useState<SortKey>('recommended')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [provider, setProvider] = useState<ProviderFilterValue>('all') // [g9] sağlayıcı filtresi
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const baseId = useId()
  const catalog = useCatalog(open)
  const privacy = usePrivacyCards(open) // §31.2 gizlilik kartları: rozet + soluklaştırma

  const list = useMemo(() => catalog.data?.models ?? (models ?? []).map(fromModelInfo), [catalog.data, models])
  const aliases = useMemo(() => catalog.data?.aliases ?? {}, [catalog.data])
  const ranks = useMemo(() => codingRanks(list), [list])
  const maxScores = useMemo(() => maxScoresOf(list), [list])
  const counts = useMemo(
    () => new Map(CATALOG_TABS.map((t) => [t.id, filterCatalog(list, { tab: t.id, query: '', aliases }).length])),
    [list, aliases],
  )
  const visible = useMemo(
    () => sortCatalog(filterCatalog(list, { tab, query, aliases }).filter((m) => providerMatches(m, provider)), sort), // [g9]
    [list, tab, query, aliases, sort, provider],
  )
  const current = list.find((m) => m.id === value) ?? models?.find((m) => m.id === value)
  const aa = catalog.data?.aa
  // §23.2: favoriler en üstte, son kullanılan 3 model hızlı erişimde, sonra sağlayıcıya göre gruplar.
  const prefs = useModelPrefs()
  const sections = useMemo(
    () => sectionModels(visible, prefs.favorites, query ? [] : prefs.recent, (m) => modelProvider(m), (p) => providerLabel(p as Parameters<typeof providerLabel>[0])),
    [visible, prefs.favorites, prefs.recent, query],
  )
  const currentProvider = modelProvider({ id: value })

  const choose = (id: string) => {
    prefs.markUsed(id)
    onChange(id)
    setOpen(false)
    setQuery('')
    setExpanded(null)
  }

  // Enter: yazılan bir takma adsa onun modeli, değilse listedeki ilk model seçilir.
  const onSearchKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== 'Enter') return
    e.preventDefault()
    const aliased = resolveModelAlias(query, aliases)
    const target = list.find((m) => m.id === aliased) ?? visible[0]
    if (target) choose(target.id)
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

  const openDialog = (dialog: Parameters<typeof openG7Dialog>[0]) => {
    setOpen(false)
    openG7Dialog(dialog)
  }

  const listError = catalog.error && !models ? errorMessage(catalog.error) : error

  return (
    <div className="model-picker" ref={ref}>
      <button
        type="button"
        className="model-trigger"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={error ?? current?.name ?? value}
      >
        <ProviderLogo provider={currentProvider} modelId={modelName(value, currentProvider)} size={14} badges={false} />
        <span className="model-trigger-name" title={current?.name ?? value}>{current?.name ?? value}</span>
        <ChevronDown size={16} className="dim" aria-hidden="true" />
      </button>

      {open && (
        <div className="menu g7-model-pop pop-in" role="dialog" aria-label="Model seç" style={popStyle}>
          <div className="g7-pick-head">
            <label htmlFor={`${baseId}-search`} className="sr-only">Model veya takma ad ara</label>
            <input
              id={`${baseId}-search`}
              className="menu-search g7-grow"
              placeholder="Model veya takma ad ara..."
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={onSearchKey}
              autoFocus
            />
            {/* [g9] sağlayıcı filtresi */}
            <ProviderFilter models={list} value={provider} onChange={setProvider} />
            <label htmlFor={`${baseId}-sort`} className="sr-only">Sırala</label>
            <select id={`${baseId}-sort`} className="g7-select" value={sort} onChange={(e) => setSort(e.target.value as SortKey)}>
              {SORT_KEYS.filter((s) => PICKER_SORTS.includes(s.id)).map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                </option>
              ))}
            </select>
            <button
              type="button"
              className="action-btn"
              onClick={() => void refresh()}
              disabled={refreshing}
              aria-label="Model listesini ve puanları yenile"
              title="Yenile"
            >
              <RefreshCw size={14} aria-hidden="true" className={refreshing ? 'g7-spin' : undefined} />
            </button>
          </div>

          <div className="g7-tabs" role="tablist" aria-label="Model filtresi">
            {CATALOG_TABS.map((t) => (
              <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} className="g7-tab" onClick={() => setTab(t.id)}>
                {t.label}
                <span className="g7-tab-count">{counts.get(t.id) ?? 0}</span>
              </button>
            ))}
          </div>

          {(listError ?? refreshError) && <p className="menu-note error-text">{listError ?? refreshError}</p>}
          {!list.length && !listError && <p className="menu-note">Modeller yükleniyor...</p>}
          {(aa?.status === 'error' || aa?.status === 'stale') && (
            <p className="menu-note">
              {aa.status === 'stale' ? 'Puanlar güncellenemedi, önbellekteki veri gösteriliyor: ' : 'Puanlar alınamadı: '}
              {aa.message}
            </p>
          )}

          <ul className="menu-list g7-model-list" aria-label="Modeller">
            {sections.flatMap((sec) => [
              <li key={`h:${sec.key}`} className="menu-group" role="presentation">
                {sec.label}
              </li>,
              ...sec.models.map((m) => {
                const isOpen = expanded === m.id
                const detailId = `${baseId}-detail-${sec.key}-${m.id}`
                const modelAliases = aliasesFor(m.id, aliases)
                const provider = modelProvider(m)
                const fav = prefs.favorites.includes(m.id)
                return (
                  <li key={`${sec.key}:${m.id}`} className={`g7-row${m.id === value ? ' selected' : ''}${isDimmed(privacy.data, m.id) ? ' kp-dim' : ''}`}>
                    <div className="g7-row-main">
                      <button type="button" className="g7-row-pick" onClick={() => choose(m.id)} aria-current={m.id === value ? 'true' : undefined}>
                        <span className="menu-check">{m.id === value ? <Check size={14} aria-hidden="true" /> : null}</span>
                        {/* §23.2: logo · model adı · bağlam · giriş/çıkış fiyatı · rozetler · yetenek ikonları */}
                        <ProviderLogo provider={provider} modelId={modelName(m.id, provider)} size={16} />
                        <span className="g7-row-text">
                          <span className="g7-row-title">
                            <span className="truncate">{m.name}</span>
                            <ModelBadges model={m} rank={ranks.get(m.id)} />
                            <PrivacyBadge modelId={m.id} cards={privacy.data} />
                          </span>
                          <span className="menu-desc">
                            {m.id}
                            {m.contextTokens ? ` · ${formatTokens(m.contextTokens)} bağlam` : ''}
                            {m.price ? ` · ${formatUsd(m.price.input)} / ${formatUsd(m.price.output)}` : ''}
                            {modelAliases.length > 0 ? ` · ${modelAliases.join(', ')}` : ''}
                          </span>
                        </span>
                        {m.aa && (
                          <span className="g7-row-scores">
                            <ScoreBar label="Kod" value={m.aa.coding} max={maxScores.coding} />
                            <ScoreBar label="Zeka" value={m.aa.intelligence} max={maxScores.intelligence} />
                          </span>
                        )}
                      </button>
                      <button
                        type="button"
                        className={`action-btn kor-fav${fav ? ' active' : ''}`}
                        onClick={() => prefs.toggleFav(m.id)}
                        aria-pressed={fav}
                        aria-label={fav ? `${m.name}: favorilerden çıkar` : `${m.name}: favorilere ekle`}
                        title={fav ? 'Favorilerden çıkar' : 'Favorilere ekle'}
                      >
                        <Star size={14} aria-hidden="true" fill={fav ? 'currentColor' : 'none'} />
                      </button>
                      <button
                        type="button"
                        className="action-btn g7-row-expand"
                        onClick={() => setExpanded(isOpen ? null : m.id)}
                        aria-expanded={isOpen}
                        aria-controls={isOpen ? detailId : undefined}
                        aria-label={`${m.name} ayrıntıları`}
                        title="Ayrıntılar"
                      >
                        <ChevronDown size={14} aria-hidden="true" className={`activity-chevron${isOpen ? ' g7-flip' : ''}`} />
                      </button>
                    </div>
                    {isOpen && <ModelDetail id={detailId} model={m} rank={ranks.get(m.id)} aliases={modelAliases} maxScores={maxScores} />}
                  </li>
                )
              }),
            ])}
            {list.length > 0 && visible.length === 0 && <li className="menu-note">Sonuç yok</li>}
          </ul>

          <div className="g7-pick-foot">
            <button type="button" className="link-btn" onClick={() => openDialog({ kind: 'compare' })}>
              <Columns3 size={14} aria-hidden="true" /> Modelleri karşılaştır
            </button>
            <button type="button" className="link-btn" onClick={() => openDialog({ kind: 'probe', models: [value] })}>
              <FlaskConical size={14} aria-hidden="true" /> Model testi
            </button>
            {aa && aa.matched > 0 && <AaAttribution compact />}
          </div>
        </div>
      )}
    </div>
  )
}
