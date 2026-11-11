// Proje grafı paneli (§7.6): canlı kuvvet yönelimli görünüm, filtreler, arama, vurgu katmanları, düğüm kartı,
// yol/etki/fark/dışa aktarma. Varsayılan ayrıntı düzeyi en yüksek dereceli 400 düğümdür; topluluk seçilince o
// topluluğun düğümleri, "Ayrıntı" artırılınca 5000'e dek düğüm çizilir.
import { Crosshair, Flame, List, Network, RefreshCw, Search, X } from 'lucide-react'
import { useEffect, useMemo, useState, useSyncExternalStore, type FormEvent } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { NodeKind, PathStep, QueryHit } from '../../../ipc-graph.ts'
import { errorMessage } from '../../api.ts'
import { useMotionStill } from '../../motion.ts'
import type { KorPanelProps } from '../registry.ts'
import { graphApi } from './bridge.ts'
import { GraphSidebar, type GraphTheme } from './GraphSidebar.tsx'
import { GraphCanvas, type Layers } from './GraphCanvas.tsx'
import { DiffTool, ExportTool, ImpactResult } from './GraphTools.tsx'
import { FindTool } from './FindTool.tsx'
import { ago, ALL_KINDS, communitySlots, indexById, KIND_LABEL, nodesForFiles, pathSegments, visibleMask } from './model.ts'
import { NodeCard } from './NodeCard.tsx'
import { changedFiles, onChangedFiles } from './session-changes.ts'
import { uiLocale } from '../../ui-locale.ts'

const DETAIL_LEVELS = [400, 1500, 5000] as const
const FLASH_MS = 2500
const LIST_MAX = 300

// Ayarlar > Animasyonlar, OS isteği, düşük güç ve ui.reduced_motion birlikte (motion.ts).
const useReducedMotion = useMotionStill

/** Saniyede bir değil, yalnızca görünür metin eskidiğinde yeniden çizmek için dakikalık saat. */
function useMinuteClock(): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const t = setTimeout(() => setNow(Date.now()), 30_000)
    return () => clearTimeout(t)
  }, [now])
  return now
}

export function GraphPanel({ sessionId }: KorPanelProps) {
  const { mutate } = useSWRConfig()
  const reducedMotion = useReducedMotion()
  const now = useMinuteClock()
  const [limit, setLimit] = useState<number>(DETAIL_LEVELS[1])
  const [community, setCommunity] = useState<number | null>(null)
  const [kinds, setKinds] = useState<ReadonlySet<NodeKind>>(() => new Set(ALL_KINDS))
  const [showTouched, setShowTouched] = useState(true)
  const [showChanged, setShowChanged] = useState(true)
  const [showHot, setShowHot] = useState(true)
  const [mode, setMode] = useState<'canvas' | 'list'>('canvas')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [pathFrom, setPathFrom] = useState<string | null>(null)
  const [path, setPath] = useState<PathStep[] | null>(null)
  const [pathError, setPathError] = useState<string | null>(null)
  const [impactId, setImpactId] = useState<string | null>(null)
  const [fitSeq, setFitSeq] = useState(0)
  const [query, setQuery] = useState('')
  const [hits, setHits] = useState<QueryHit[] | null>(null)
  const [queryError, setQueryError] = useState<string | null>(null)
  const [updating, setUpdating] = useState(false)
  const [updateError, setUpdateError] = useState<string | null>(null)
  const [touchedNow, setTouchedNow] = useState<ReadonlyMap<string, number>>(new Map())
  const [touchedLive, setTouchedLive] = useState<ReadonlySet<string>>(new Set())
  const [hiddenCommunities, setHiddenCommunities] = useState<ReadonlySet<number>>(new Set())
  const [graphTheme, setGraphTheme] = useState<GraphTheme>('system')

  const status = useSWR('graph:status', () => graphApi.status(), { revalidateOnFocus: false })
  const refreshStatus = status.mutate
  const snapshot = useSWR(status.data ? ['graph:snapshot', limit, community] : null, () => graphApi.snapshot(limit, community), {
    revalidateOnFocus: false,
    keepPreviousData: true,
  })
  const hotspots = useSWR(status.data ? 'graph:hotspots' : null, () => graphApi.hotspots(30), { revalidateOnFocus: false })
  const changed = useSyncExternalStore(onChangedFiles, () => changedFiles(sessionId))

  // Canlı yenileme: graf değişince durum, çizim ve açık sorgular yeniden okunur.
  useEffect(
    () =>
      graphApi.onUpdated((e) => {
        void refreshStatus()
        if (!e.stats.changed) return
        void mutate((key) => Array.isArray(key) && typeof key[0] === 'string' && key[0].startsWith('graph:'))
        void mutate('graph:hotspots')
      }),
    [mutate, refreshStatus],
  )

  // Ajan dokundukça: o dosyaların düğümleri yanıp söner; süre bitince işaret kalkar (tek seferlik zamanlayıcı).
  useEffect(
    () =>
      graphApi.onTouched((e) => {
        const at = performance.now()
        setTouchedNow((prev) => {
          const next = new Map([...prev].filter(([, t]) => at - t < FLASH_MS))
          for (const f of e.files) next.set(f, at)
          return next
        })
        setTouchedLive((prev) => new Set([...prev, ...e.files]))
        setTimeout(() => setTouchedNow((prev) => new Map([...prev].filter(([, t]) => performance.now() - t < FLASH_MS))), FLASH_MS + 50)
      }),
    [],
  )

  const snap = snapshot.data
  const index = useMemo(() => (snap ? indexById(snap) : new Map<string, number>()), [snap])
  const mask = useMemo(() => {
    if (!snap) return new Uint8Array()
    const m = visibleMask(snap.nodes, { kinds, community: null })
    if (hiddenCommunities.size > 0) snap.nodes.forEach((n, i) => { if (hiddenCommunities.has(n.community)) m[i] = 0 })
    return m
  }, [snap, kinds, hiddenCommunities])
  const slots = useMemo(() => (snap ? communitySlots(snap.nodes, snap.communities) : new Int32Array()), [snap])
  const visibleEdges = useMemo(() => (snap ? snap.edges.reduce((k, [a, b]) => k + (mask[a] && mask[b] ? 1 : 0), 0) : 0), [snap, mask])

  const layers = useMemo<Layers>(() => {
    const nodes = snap?.nodes ?? []
    const live = new Map<number, number>()
    if (showTouched) {
      for (const [file, t] of touchedNow) for (const i of nodesForFiles(nodes, [file])) live.set(i, t)
    }
    const hot = new Set<number>()
    if (showHot) for (const h of hotspots.data ?? []) {
      const i = index.get(h.node.id)
      if (i !== undefined) hot.add(i)
    }
    return {
      touchedNow: live,
      touchedSession: showTouched ? nodesForFiles(nodes, [...(status.data?.touched ?? []), ...touchedLive]) : new Set(),
      changed: showChanged ? nodesForFiles(nodes, changed) : new Set(),
      hotspots: hot,
    }
  }, [snap, index, touchedNow, touchedLive, status.data?.touched, changed, hotspots.data, showTouched, showChanged, showHot])

  const selectedIndex = selectedId !== null ? (index.get(selectedId) ?? null) : null
  const pathFromIndex = pathFrom !== null ? (index.get(pathFrom) ?? null) : null
  const segments = useMemo(() => (path ? pathSegments(path, index) : []), [path, index])

  const select = (id: string | null) => {
    if (id !== null && pathFrom !== null && id !== pathFrom) {
      const from = pathFrom
      setPathFrom(null)
      setPathError(null)
      graphApi
        .path(from, id)
        .then((steps) => {
          setPath(steps)
          if (steps === null) setPathError('İki düğüm arasında yol yok.')
        })
        .catch((err: unknown) => setPathError(errorMessage(err)))
    }
    setSelectedId(id)
  }

  const runQuery = async (e: FormEvent) => {
    e.preventDefault()
    const q = query.trim()
    if (!q) return
    setQueryError(null)
    try {
      setHits(await graphApi.query(q, 20))
    } catch (err) {
      setQueryError(errorMessage(err))
    }
  }

  const update = async () => {
    setUpdating(true)
    setUpdateError(null)
    try {
      await graphApi.update()
    } catch (err) {
      setUpdateError(errorMessage(err))
    } finally {
      setUpdating(false)
    }
  }

  const st = status.data?.status
  const visibleCount = mask.reduce((a, b) => a + b, 0)
  const listRows = useMemo(() => {
    if (!snap) return []
    return snap.nodes
      .map((n, i) => ({ n, i }))
      .filter(({ i }) => mask[i])
      .sort((a, b) => b.n.degree - a.n.degree)
      .slice(0, LIST_MAX)
  }, [snap, mask])

  if (status.error) {
    return (
      <div className="kg kg-empty" role="alert">
        <Network size={28} aria-hidden="true" />
        <p>Graf açılamadı</p>
        <p className="muted">{errorMessage(status.error)}</p>
        <button type="button" className="btn small" onClick={() => void status.mutate()} data-kor-cmd="graph.status">
          <RefreshCw size={14} aria-hidden="true" />
          Yeniden dene
        </button>
      </div>
    )
  }
  if (!st) {
    return (
      <div className="kg kg-empty" role="status" data-kor-cmd="graph.status">
        <Network size={28} aria-hidden="true" />
        <p>Proje grafı kuruluyor…</p>
        <p className="muted">İlk tarama büyük projelerde birkaç saniye sürer; sonraki güncellemeler artımlıdır.</p>
      </div>
    )
  }

  return (
    <div className="kg" data-kor-event="kor:graph/updated">
      <div className="kg-status" role="status" data-kor-cmd="graph.status">
        <span>{`${st.nodes.toLocaleString(uiLocale())} düğüm · ${st.edges.toLocaleString(uiLocale())} kenar · ${st.communities} topluluk`}</span>
        <span className="muted">{st.last_update === null ? 'Henüz güncellenmedi' : `${ago(st.last_update, now)} güncellendi`}</span>
        {status.data && status.data.packages > 0 && <span className="muted">{`${status.data.packages} paket`}</span>}
        <button type="button" className="btn small kg-push" onClick={() => void update()} disabled={updating} data-kor-cmd="graph.update" title="Projeyi baştan tara">
          <RefreshCw size={14} aria-hidden="true" />
          {updating ? 'Güncelleniyor…' : 'Güncelle'}
        </button>
      </div>
      {updateError && <p className="error-text" role="alert">{updateError}</p>}

      <form className="kg-row kg-search" onSubmit={(e) => void runQuery(e)} role="search">
        <label className="cdx-search">
          <Search size={13} aria-hidden="true" />
          <input
            type="search"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value)
              if (!e.target.value) setHits(null)
            }}
            placeholder="Grafta ara: ödeme hatası nerede loglanıyor"
            aria-label="Grafta ara"
            spellCheck={false}
            maxLength={500}
            data-kor-cmd="graph.query"
          />
        </label>
        <div className="kg-seg" role="group" aria-label="Görünüm">
          <button type="button" className={`action-btn${mode === 'canvas' ? ' active' : ''}`} aria-pressed={mode === 'canvas'} onClick={() => setMode('canvas')} aria-label="Grafik görünümü" title="Grafik görünümü">
            <Network size={14} aria-hidden="true" />
          </button>
          <button type="button" className={`action-btn${mode === 'list' ? ' active' : ''}`} aria-pressed={mode === 'list'} onClick={() => setMode('list')} aria-label="Liste görünümü" title="Liste görünümü (klavye)">
            <List size={14} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" onClick={() => setFitSeq((s) => s + 1)} aria-label={selectedId ? 'Seçili düğüme odaklan' : 'Grafı sığdır'} title={selectedId ? 'Seçili düğüme odaklan' : 'Grafı sığdır'} disabled={mode !== 'canvas'}>
            <Crosshair size={14} aria-hidden="true" />
          </button>
        </div>
      </form>
      {queryError && <p className="error-text" role="alert">{queryError}</p>}
      {hits && (
        <div className="kg-hits">
          {hits.length === 0 ? (
            <p className="muted">Eşleşen düğüm yok.</p>
          ) : (
            <ul className="kg-plain" aria-label="Arama sonuçları">
              {hits.map((h) => (
                <li key={h.node.id}>
                  <button type="button" className="kg-link" onClick={() => select(h.node.id)} title={h.node.file ?? h.node.id}>
                    <span translate="no">{h.node.name}</span>
                    <span className="kg-link-meta">
                      {KIND_LABEL[h.node.kind]}
                      {!index.has(h.node.id) && ' · çizimde değil'}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      <div className="kg-filters">
        <label className="kg-field">
          <span>Topluluk</span>
          <select value={community ?? ''} onChange={(e) => setCommunity(e.target.value === '' ? null : Number(e.target.value))} data-kor-cmd="graph.snapshot">
            <option value="">Tümü (en bağlantılılar)</option>
            {(snap?.communities ?? []).map((c) => (
              <option key={c.id} value={c.id} translate="no">
                {`${c.label} (${c.size})`}
              </option>
            ))}
          </select>
        </label>
        <label className="kg-field">
          <span>Ayrıntı</span>
          <select value={limit} onChange={(e) => setLimit(Number(e.target.value))} data-kor-cmd="graph.snapshot">
            {DETAIL_LEVELS.map((l) => (
              <option key={l} value={l}>
                {`${l.toLocaleString(uiLocale())} düğüm`}
              </option>
            ))}
          </select>
        </label>
        <details className="kg-kinds">
          <summary>{kinds.size === ALL_KINDS.length ? 'Tüm türler' : `${kinds.size} tür`}</summary>
          <fieldset>
            <legend className="sr-only">Düğüm türleri</legend>
            {ALL_KINDS.map((k) => (
              <label key={k} className="kg-check">
                <input
                  type="checkbox"
                  checked={kinds.has(k)}
                  onChange={(e) =>
                    setKinds((prev) => {
                      const next = new Set(prev)
                      if (e.target.checked) next.add(k)
                      else next.delete(k)
                      return next
                    })
                  }
                />
                {KIND_LABEL[k]}
                {st.nodes_by_kind[k] !== undefined && <span className="muted">{` ${st.nodes_by_kind[k]}`}</span>}
              </label>
            ))}
          </fieldset>
        </details>
      </div>

      <div className="kg-layers" role="group" aria-label="Vurgu katmanları" data-kor-event="kor:graph/touched">
        <label className="kg-check">
          <input type="checkbox" checked={showTouched} onChange={(e) => setShowTouched(e.target.checked)} />
          <span className="kg-swatch kg-swatch-touched" aria-hidden="true" />
          Ajan burada
        </label>
        <label className="kg-check">
          <input type="checkbox" checked={showChanged} onChange={(e) => setShowChanged(e.target.checked)} />
          <span className="kg-swatch kg-swatch-changed" aria-hidden="true" />
          {`Bu oturumda değişen (${changed.size})`}
        </label>
        <label className="kg-check" data-kor-cmd="graph.hotspots">
          <input type="checkbox" checked={showHot} onChange={(e) => setShowHot(e.target.checked)} />
          <Flame size={12} aria-hidden="true" className="kg-hot-icon" />
          Hotspot
        </label>
      </div>

      {pathFrom !== null && (
        <div className="kg-banner" role="status">
          <span>
            Yol başlangıcı: <span translate="no">{pathFrom}</span>. Şimdi ikinci düğümü seç.
          </span>
          <button type="button" className="action-btn" onClick={() => setPathFrom(null)} aria-label="Yol seçimini iptal et" title="İptal">
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      )}
      {pathError && <p className="error-text" role="alert">{pathError}</p>}
      {path && path.length > 0 && (
        <div className="kg-banner" role="status">
          <span translate="no" className="kg-path-steps">
            {path.map((s) => s.node.name).join(' → ')}
          </span>
          <button type="button" className="action-btn" onClick={() => setPath(null)} aria-label="Yolu temizle" title="Yolu temizle">
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      )}

      <div className="kg-view">
      <div className="kg-stage">
        {snapshot.error ? (
          <p className="error-text pad" role="alert">{errorMessage(snapshot.error)}</p>
        ) : !snap ? (
          <p className="muted pad" role="status">Çizim verisi okunuyor…</p>
        ) : snap.nodes.length === 0 ? (
          <div className="kg-empty">
            <Network size={28} aria-hidden="true" />
            <p>Grafta düğüm yok</p>
            <p className="muted">Desteklenen dilde kaynak dosya bulunamadı ya da .gitignore hepsini dışarıda bırakıyor.</p>
          </div>
        ) : mode === 'canvas' ? (
          <GraphCanvas
            snapshot={snap}
            mask={mask}
            layers={layers}
            selected={selectedIndex}
            pathFrom={pathFromIndex}
            path={segments}
            onSelect={(i) => select(i === null ? null : (snap.nodes[i]?.id ?? null))}
            reducedMotion={reducedMotion}
            fitSeq={fitSeq}
            slots={slots}
            theme={graphTheme}
          />
        ) : (
          <ul className="kg-list" aria-label="Graf düğümleri (dereceye göre)">
            {listRows.map(({ n, i }) => (
              <li key={n.id}>
                <button type="button" className="kg-link" aria-current={selectedId === n.id} onClick={() => select(n.id)} title={n.file ?? n.id}>
                  <span translate="no">{n.name}</span>
                  <span className="kg-link-meta">
                    {`${KIND_LABEL[n.kind]} · ${n.degree}`}
                    {layers.touchedSession.has(i) && ' · ajan'}
                    {layers.changed.has(i) && ' · değişti'}
                    {layers.hotspots.has(i) && ' · hotspot'}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      {snap && snap.nodes.length > 0 && (
        <GraphSidebar
          snapshot={snap}
          slots={slots}
          hidden={hiddenCommunities}
          onHidden={setHiddenCommunities}
          selected={selectedIndex}
          onSelect={(i) => {
            select(snap.nodes[i]?.id ?? null)
            setFitSeq((s) => s + 1)
          }}
          theme={graphTheme}
          onTheme={setGraphTheme}
          visibleNodes={visibleCount}
          visibleEdges={visibleEdges}
        />
      )}
      </div>
      {snap && (
        <p className="muted kg-count">
          {`${visibleCount.toLocaleString(uiLocale())} / ${snap.total.toLocaleString(uiLocale())} düğüm gösteriliyor`}
          {snap.truncated && ' · ayrıntıyı artır ya da topluluk seç'}
        </p>
      )}

      {selectedId !== null && (
        <NodeCard
          key={selectedId}
          id={selectedId}
          node={selectedIndex !== null ? (snap?.nodes[selectedIndex] ?? null) : null}
          onSelect={select}
          onClose={() => setSelectedId(null)}
          onPath={(id) => {
            setPath(null)
            setPathError(null)
            setPathFrom((p) => (p === id ? null : id))
          }}
          onImpact={setImpactId}
          pathPending={pathFrom === selectedId}
        />
      )}
      {impactId !== null && <ImpactResult id={impactId} onSelect={select} onClose={() => setImpactId(null)} />}

      <details className="kg-more-tools">
        <summary>Kodda bul</summary>
        <FindTool onSelect={select} />
      </details>
      <details className="kg-more-tools">
        <summary>Dışa aktar ve karşılaştır</summary>
        <ExportTool />
        <DiffTool onSelect={select} />
      </details>
    </div>
  )
}
