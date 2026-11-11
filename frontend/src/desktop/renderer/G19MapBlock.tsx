// [g19] ```map ve ```geojson blokları (#56-70): paketli vektör altlık üzerinde choropleth, işaretçi, ısı noktası,
//      rota ve çokgen katmanları; bölgeye tıklayınca değer, lejant + ölçek çubuğu, minyatür/büyük görünüm, katman
//      anahtarı, açık/koyu tema, elle çokgen çizme, ondalık/DMS koordinat kopyalama, ekran okuyucu tablosu ve g17
//      dışa aktarma menüsü (PNG/SVG). Güvenlik: ağ yok (dış kiremit kapalı); model verisi DOM'a yalnızca React metni
//      ve sayısal öznitelik olarak girer, ham HTML/SVG enjeksiyonu yok. Ağır kütüphane + veri tembel pakettedir.
import { Layers, Map as MapIcon, Maximize2, Minimize2, Pentagon, Table2, Trash, Undo2 } from 'lucide-react'
import { useDeferredValue, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore, type MouseEvent } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { G17DiagramExport } from './G17Blocks.tsx'
import {
  colorScale,
  draftToGeoJson,
  formatCoord,
  formatKm,
  formatValue,
  MAP_THEMES,
  parseGeoJsonBlock,
  parseMapBlock,
  scaleBar,
  type ColorScale,
  type CoordFormat,
  type G19MapApi,
  type LonLat,
  type MapModel,
  type MapScene,
  type MapTheme,
  type ParseResult,
} from './g19-geo.ts'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { loadRichBundle } from './rich-lazy.ts'
import { withinRichSizeLimit } from './rich-sanitize.ts'
import { BlockFrame, RawFallback } from './rich-shared.tsx'

const FONT = "'Segoe UI', system-ui, -apple-system, sans-serif"
const TILE_NOTE = 'Dış kiremitler kapalı (uygulama ağ erişimine izin vermez); yalnızca paketli vektör altlık kullanılır.'
/** Ekran okuyucu tablosunda bölüm başına en fazla satır (çok büyük veride DOM şişmesin). */
const MAX_TABLE_ROWS = 200
const MAX_DRAFT = 500
const MAX_LABELS = 25

type LayerId = 'regions' | 'markers' | 'heat' | 'routes' | 'polygons' | 'grid' | 'legend'
const LAYER_LABELS: Record<LayerId, string> = {
  regions: 'Bölge değerleri',
  markers: 'İşaretçiler',
  heat: 'Isı noktaları',
  routes: 'Rotalar',
  polygons: 'Çokgenler',
  grid: 'Enlem/boylam ağı',
  legend: 'Lejant ve ölçek',
}

const KINDS = ['country', 'marker', 'route', 'polygon'] as const
type SelKind = (typeof KINDS)[number]
type Selection = { kind: SelKind; index: number } | null

function parseSelection(key: string | null | undefined): Selection {
  const [kind, i] = (key ?? '').split(':')
  const index = Number(i)
  const found = KINDS.find((k) => k === kind)
  return found && Number.isInteger(index) ? { kind: found, index } : null
}

// ---------- Tema (uygulama temasını izler) ----------

const darkQuery = () => window.matchMedia('(prefers-color-scheme: dark)')
function subscribeScheme(onChange: () => void): () => void {
  const q = darkQuery()
  q.addEventListener('change', onChange)
  return () => q.removeEventListener('change', onChange)
}
const useSystemDark = () => useSyncExternalStore(subscribeScheme, () => darkQuery().matches)

// ---------- Yardımcılar ----------

function summaryOf(model: MapModel, scene: MapScene): string {
  const parts = [
    [scene.countries.filter((c) => c.value !== undefined).length, 'bölge değeri'],
    [scene.markers.length, 'işaretçi'],
    [scene.heat.length, 'ısı noktası'],
    [scene.routes.length, 'rota'],
    [scene.polygons.length, 'çokgen'],
  ] as const
  const text = parts.filter(([n]) => n > 0).map(([n, label]) => `${n} ${label}`)
  return `${model.title ?? 'Harita'}: ${text.join(', ')}. Ayrıntılar tablo görünümündedir.`
}

// ---------- Blok ----------

function G19MapBlock({ code, language }: RichBlockProps) {
  // Akış sürerken her parçada yeniden çizim düşük öncelikte yapılır.
  const source = useDeferredValue(code)
  const parsed = useMemo<ParseResult>(() => {
    if (!withinRichSizeLimit(source)) return { ok: false, error: 'Harita verisi görüntülenemeyecek kadar büyük.' }
    return language.trim().toLowerCase() === 'geojson' ? parseGeoJsonBlock(source) : parseMapBlock(source)
  }, [source, language])
  const [api, setApi] = useState<G19MapApi | null>(() => window.__g19Map ?? null)
  const [loadError, setLoadError] = useState(false)

  useEffect(() => {
    if (api) return
    let cancelled = false
    loadRichBundle<G19MapApi>('./g19-map.js', '__g19Map').then(
      (loaded) => {
        if (!cancelled) setApi(loaded)
      },
      (err: unknown) => {
        console.warn('Harita paketi yüklenemedi:', err)
        if (!cancelled) setLoadError(true)
      },
    )
    return () => {
      cancelled = true
    }
  }, [api])

  const built = useMemo(() => {
    if (!api || !parsed.ok) return null
    try {
      return { scene: api.buildScene(parsed.model) }
    } catch (err) {
      console.warn('Harita çizilemedi:', err)
      return { error: 'Harita çizilemedi.' }
    }
  }, [api, parsed])

  if (!parsed.ok) return <RawFallback code={code} note={parsed.error} />
  if (loadError) return <RawFallback code={code} note="Harita kütüphanesi yüklenemedi." />
  if (!built) {
    return (
      <BlockFrame icon={<MapIcon size={14} aria-hidden="true" />} label="Harita" className="g19-map">
        <p className="g19-muted" role="status">
          Harita yükleniyor…
        </p>
      </BlockFrame>
    )
  }
  if ('error' in built) return <RawFallback code={code} note={built.error} />
  return <MapView scene={built.scene} model={parsed.model} source={source} />
}

interface MapViewProps {
  scene: MapScene
  model: MapModel
  source: string
}

function MapView({ scene, model, source }: MapViewProps) {
  const hostRef = useRef<HTMLDivElement>(null)
  const uid = `g19${useId().replace(/[^a-zA-Z0-9_-]/g, '')}`
  const systemDark = useSystemDark()
  const [themeChoice, setThemeChoice] = useState<'auto' | 'light' | 'dark'>('auto')
  const theme = MAP_THEMES[themeChoice === 'auto' ? (systemDark ? 'dark' : 'light') : themeChoice]
  const [expanded, setExpanded] = useState(false)
  const [panel, setPanel] = useState(false)
  const [showTable, setShowTable] = useState(false)
  const [format, setFormat] = useState<CoordFormat>('decimal')
  const [hidden, setHidden] = useState<ReadonlySet<LayerId>>(() => new Set())
  const [selected, setSelected] = useState<Selection>(null)
  const [clicked, setClicked] = useState<LonLat | null>(null)
  const [drawing, setDrawing] = useState(false)
  const [draft, setDraft] = useState<LonLat[]>([])

  const scale = colorScale([...scene.countries.flatMap((c) => c.value ?? []), ...scene.polygons.flatMap((p) => p.value ?? [])], theme.ramp)
  const bar = scene.kmPerPx ? scaleBar(scene.kmPerPx) : null
  const available: LayerId[] = [
    ...(scene.countries.some((c) => c.value !== undefined) ? (['regions'] as const) : []),
    ...(scene.markers.length ? (['markers'] as const) : []),
    ...(scene.heat.length ? (['heat'] as const) : []),
    ...(scene.routes.length ? (['routes'] as const) : []),
    ...(scene.polygons.length ? (['polygons'] as const) : []),
    'grid',
    ...(scale || bar ? (['legend'] as const) : []),
  ]
  const show = (id: LayerId) => !hidden.has(id)
  const toggleLayer = (id: LayerId) =>
    setHidden((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  const onMapClick = (e: MouseEvent<SVGSVGElement>) => {
    if (!expanded) {
      setExpanded(true)
      return
    }
    const ctm = e.currentTarget.getScreenCTM()
    const pt = ctm ? new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse()) : null
    const lonLat = pt ? scene.invert(pt.x, pt.y) : null
    if (drawing) {
      if (lonLat) setDraft((d) => (d.length >= MAX_DRAFT ? d : [...d, lonLat]))
      return
    }
    setClicked(lonLat)
    const target = e.target instanceof Element ? e.target.closest('[data-k]') : null
    setSelected(parseSelection(target?.getAttribute('data-k')))
  }

  const summary = summaryOf(model, scene)
  const unit = model.unit
  const selCountry = selected?.kind === 'country' ? scene.countries[selected.index] : undefined
  const selPolygon = selected?.kind === 'polygon' ? scene.polygons[selected.index] : undefined
  const selRoute = selected?.kind === 'route' ? scene.routes[selected.index] : undefined
  const selMarker = selected?.kind === 'marker' ? scene.markers[selected.index] : undefined
  const maxHeat = Math.max(1, ...scene.heat.map((h) => h.value ?? 1))
  const draftPoints = draft.flatMap((p) => {
    const xy = scene.project(p)
    return xy ? [xy] : []
  })
  const labels = expanded && scene.markers.length <= MAX_LABELS

  let info = ''
  if (selCountry) info = `${selCountry.name}: ${selCountry.value !== undefined ? formatValue(selCountry.value, unit) : 'veri yok'}`
  else if (selMarker) info = `${selMarker.label ?? 'İşaretçi'}${selMarker.value !== undefined ? `: ${formatValue(selMarker.value, unit)}` : ''}`
  else if (selRoute) info = `${selRoute.label ?? 'Rota'}: ${selRoute.coords.length} nokta, yaklaşık ${formatKm(selRoute.km)}`
  else if (selPolygon) info = `${selPolygon.label ?? 'Çokgen'}: ${selPolygon.rings[0]?.length ?? 0} köşe${selPolygon.value !== undefined ? `, ${formatValue(selPolygon.value, unit)}` : ''}`
  const coord: [number, number] | null = selMarker ? [selMarker.lat, selMarker.lon] : clicked ? [clicked[1], clicked[0]] : null

  const tools = (
    <>
      <button type="button" className={`action-btn${panel ? ' active' : ''}`} onClick={() => setPanel((p) => !p)} aria-pressed={panel} aria-label="Katmanlar ve tema" title="Katmanlar ve tema">
        <Layers size={15} aria-hidden="true" />
      </button>
      <button type="button" className={`action-btn${showTable ? ' active' : ''}`} onClick={() => setShowTable((s) => !s)} aria-pressed={showTable} aria-label="Tablo görünümü" title="Tablo görünümü">
        <Table2 size={15} aria-hidden="true" />
      </button>
      <button
        type="button"
        className={`action-btn${drawing ? ' active' : ''}`}
        onClick={() => {
          setDrawing((d) => !d)
          setExpanded(true)
        }}
        aria-pressed={drawing}
        aria-label="Çokgen çiz"
        title="Çokgen çiz"
      >
        <Pentagon size={15} aria-hidden="true" />
      </button>
      <button
        type="button"
        className={`action-btn g19-tool-text${format === 'dms' ? ' active' : ''}`}
        onClick={() => setFormat((f) => (f === 'dms' ? 'decimal' : 'dms'))}
        aria-pressed={format === 'dms'}
        aria-label="Koordinatları derece-dakika-saniye (DMS) göster"
        title="Koordinat biçimi: ondalık / DMS"
      >
        DMS
      </button>
      <button type="button" className="action-btn" onClick={() => setExpanded((x) => !x)} aria-label={expanded ? 'Haritayı küçült' : 'Haritayı büyüt'} title={expanded ? 'Küçült' : 'Büyüt'}>
        {expanded ? <Minimize2 size={15} aria-hidden="true" /> : <Maximize2 size={15} aria-hidden="true" />}
      </button>
      <G17DiagramExport hostRef={hostRef} source={source} sourceLabel="Harita" />
    </>
  )

  return (
    <BlockFrame icon={<MapIcon size={14} aria-hidden="true" />} label={model.title ?? 'Harita'} tools={tools} className="g19-map">
      {scene.unmatched.length > 0 && (
        <p className="g8-note" role="note">
          Eşleşmeyen bölgeler: {scene.unmatched.slice(0, 8).join(', ')}
          {scene.unmatched.length > 8 ? ` (+${scene.unmatched.length - 8})` : ''}. Ülke adı, ISO alfa-2 ya da ISO sayısal kod kullanın.
        </p>
      )}
      {model.skipped > 0 && (
        <p className="g8-note" role="note">
          {model.skipped} geçersiz girdi atlandı.
        </p>
      )}
      {model.tilesRequested && (
        <p className="g8-note" role="note">
          {TILE_NOTE}
        </p>
      )}
      {panel && (
        <div className="g19-panel">
          <fieldset className="g19-fieldset">
            <legend>Katmanlar</legend>
            {available.map((id) => (
              <label key={id} className="g19-check">
                <input type="checkbox" checked={show(id)} onChange={() => toggleLayer(id)} />
                {LAYER_LABELS[id]}
              </label>
            ))}
          </fieldset>
          <fieldset className="g19-fieldset">
            <legend>Tema</legend>
            {(
              [
                ['auto', 'Uygulamayla aynı'],
                ['light', 'Açık'],
                ['dark', 'Koyu'],
              ] as const
            ).map(([value, label]) => (
              <label key={value} className="g19-check">
                <input type="radio" name={`${uid}-theme`} checked={themeChoice === value} onChange={() => setThemeChoice(value)} />
                {label}
              </label>
            ))}
          </fieldset>
          <p className="g19-muted">Altlık: paketli Natural Earth 1:110m ülke sınırları. {TILE_NOTE}</p>
        </div>
      )}
      <div ref={hostRef} className={`g19-host${expanded ? ' g19-expanded' : ''}${drawing && expanded ? ' g19-drawing' : ''}`}>
        <MapSvg
          uid={uid}
          scene={scene}
          theme={theme}
          scale={scale}
          unit={unit}
          summary={summary}
          show={show}
          labels={labels}
          maxHeat={maxHeat}
          bar={show('legend') ? bar : null}
          legend={show('legend') ? scale : null}
          selectedPath={selCountry?.d ?? selPolygon?.d ?? selRoute?.d ?? null}
          selectedMarker={selMarker ? [selMarker.x, selMarker.y] : null}
          draftPoints={draftPoints}
          onClick={onMapClick}
        />
      </div>
      <div className="g19-info" role="status" aria-live="polite">
        {drawing && expanded ? (
          <>
            <span>Çizim: haritaya tıklayarak köşe ekleyin ({draft.length} köşe).</span>
            <button type="button" className="action-btn" onClick={() => setDraft((d) => d.slice(0, -1))} disabled={draft.length === 0} aria-label="Son köşeyi geri al" title="Geri al">
              <Undo2 size={15} aria-hidden="true" />
            </button>
            <button type="button" className="action-btn" onClick={() => setDraft([])} disabled={draft.length === 0} aria-label="Çizimi temizle" title="Temizle">
              <Trash size={15} aria-hidden="true" />
            </button>
            {draft.length >= 3 && <CopyButton label="Çokgeni GeoJSON olarak kopyala" getText={() => draftToGeoJson(draft)} />}
            {draft.length >= 3 && (
              <CopyButton
                label="Köşe koordinatlarını kopyala"
                getText={() => draft.map(([lon, lat]) => formatCoord(lat, lon, format)).join('\n')}
                icon={<span className="g19-tool-text">{format === 'dms' ? 'DMS' : '0.0'}</span>}
              />
            )}
          </>
        ) : (
          <>
            {info && <span className="g19-info-main">{info}</span>}
            {coord && (
              <>
                <span className="g19-coord">{formatCoord(coord[0], coord[1], format)}</span>
                <CopyButton label="Koordinatı kopyala" getText={() => formatCoord(coord[0], coord[1], format)} />
              </>
            )}
            {!info && !coord && <span className="g19-muted">{expanded ? 'Değerini görmek için bir bölgeye ya da işaretçiye tıklayın.' : 'Büyütmek için haritaya tıklayın.'}</span>}
          </>
        )}
      </div>
      <MapTables
        scene={scene}
        model={model}
        format={format}
        visible={showTable}
        onSelect={(sel) => {
          setSelected(sel)
          setClicked(null)
          setExpanded(true)
        }}
      />
    </BlockFrame>
  )
}

// ---------- SVG ----------

interface MapSvgProps {
  uid: string
  scene: MapScene
  theme: MapTheme
  scale: ColorScale | null
  legend: ColorScale | null
  unit?: string
  summary: string
  show: (id: LayerId) => boolean
  labels: boolean
  maxHeat: number
  bar: { km: number; px: number } | null
  selectedPath: string | null
  selectedMarker: [number, number] | null
  draftPoints: [number, number][]
  onClick: (e: MouseEvent<SVGSVGElement>) => void
}

/** Harita SVG'si. Renkler sunum özniteliğidir (CSS değil): dışa aktarılan SVG/PNG temayla birebir aynı görünür. */
function MapSvg({ uid, scene, theme, scale, legend, unit, summary, show, labels, maxHeat, bar, selectedPath, selectedMarker, draftPoints, onClick }: MapSvgProps) {
  const { width: W, height: H } = scene
  const legendH = legend ? 34 + legend.steps.length * 18 : 0
  const draftAttr = draftPoints.map(([x, y]) => `${x},${y}`).join(' ')
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width={W} height={H} role="img" aria-labelledby={`${uid}-t`} fontFamily={FONT} onClick={onClick}>
      <title id={`${uid}-t`}>{summary}</title>
      <defs>
        <radialGradient id={`${uid}-heat`}>
          <stop offset="0" stopColor={theme.accent} stopOpacity={0.85} />
          <stop offset="1" stopColor={theme.accent} stopOpacity={0} />
        </radialGradient>
      </defs>
      {scene.sphere ? <path d={scene.sphere} fill={theme.ocean} /> : <rect width={W} height={H} fill={theme.ocean} />}
      {show('grid') && <path d={scene.graticule} fill="none" stroke={theme.grid} strokeWidth={0.7} pointerEvents="none" />}
      <g stroke={theme.border} strokeWidth={0.5} strokeLinejoin="round">
        {scene.countries.map((c, i) =>
          // Görünüm dışında kırpılan ülkenin yolu boştur; çizilmez.
          c.d ? (
            <path key={c.id} d={c.d} data-k={`country:${i}`} fill={c.value !== undefined && scale && show('regions') ? scale.color(c.value) : theme.land}>
              <title>{c.value !== undefined ? `${c.name}: ${formatValue(c.value, unit)}` : c.name}</title>
            </path>
          ) : null,
        )}
      </g>
      {show('polygons') &&
        scene.polygons.map((p, i) => (
          <path
            key={i}
            d={p.d}
            data-k={`polygon:${i}`}
            fill={p.value !== undefined && scale ? scale.color(p.value) : theme.accent}
            fillOpacity={p.value !== undefined && scale ? 0.9 : 0.25}
            stroke={theme.accent}
            strokeWidth={1.5}
          >
            <title>{p.label ?? 'Çokgen'}</title>
          </path>
        ))}
      {show('heat') && (
        <g pointerEvents="none">
          {scene.heat.map((h, i) => (
            <circle key={i} cx={h.x} cy={h.y} r={22} fill={`url(#${uid}-heat)`} opacity={0.3 + (0.7 * (h.value ?? 1)) / maxHeat} />
          ))}
        </g>
      )}
      {show('routes') &&
        scene.routes.map((r, i) => (
          <path key={i} d={r.d} data-k={`route:${i}`} fill="none" stroke={theme.accent} strokeWidth={3} strokeLinecap="round" strokeLinejoin="round">
            <title>{`${r.label ?? 'Rota'}: yaklaşık ${formatKm(r.km)}`}</title>
          </path>
        ))}
      {selectedPath && <path d={selectedPath} fill="none" stroke={theme.text} strokeWidth={2} pointerEvents="none" />}
      {show('markers') &&
        scene.markers.map((m, i) => (
          <g key={i} data-k={`marker:${i}`}>
            <circle cx={m.x} cy={m.y} r={6} fill={theme.accent} stroke={theme.halo} strokeWidth={1.5} />
            {labels && m.label && (
              <text x={m.x + 9} y={m.y + 4} fontSize={13} fill={theme.text} stroke={theme.halo} strokeWidth={3} paintOrder="stroke">
                {m.label}
              </text>
            )}
            <title>{m.label ?? 'İşaretçi'}</title>
          </g>
        ))}
      {selectedMarker && <circle cx={selectedMarker[0]} cy={selectedMarker[1]} r={10} fill="none" stroke={theme.text} strokeWidth={2} pointerEvents="none" />}
      {draftPoints.length > 0 && (
        <g pointerEvents="none" stroke={theme.text} strokeWidth={1.5}>
          {draftPoints.length >= 3 ? (
            <polygon points={draftAttr} fill={theme.text} fillOpacity={0.12} strokeDasharray="6 4" />
          ) : (
            <polyline points={draftAttr} fill="none" strokeDasharray="6 4" />
          )}
          {draftPoints.map(([x, y], i) => (
            <circle key={i} cx={x} cy={y} r={4} fill={theme.halo} />
          ))}
        </g>
      )}
      {legend && (
        <g transform={`translate(14 ${H - 14 - legendH})`} pointerEvents="none">
          <rect width={190} height={legendH} rx={6} fill={theme.halo} fillOpacity={0.88} />
          <text x={10} y={20} fontSize={13} fontWeight={600} fill={theme.text}>
            {unit ? `Değer (${unit})` : 'Değer'}
          </text>
          {legend.steps.map((s, i) => (
            <g key={i} transform={`translate(10 ${30 + i * 18})`}>
              <rect width={16} height={12} rx={2} fill={s.color} />
              <text x={24} y={11} fontSize={12} fill={theme.text}>
                {s.from === s.to ? formatValue(s.from) : `${formatValue(s.from)} – ${formatValue(s.to)}`}
              </text>
            </g>
          ))}
        </g>
      )}
      {bar && (
        <g transform={`translate(${W - 20 - bar.px} ${H - 18})`} pointerEvents="none">
          <rect x={-8} y={-24} width={bar.px + 16} height={32} rx={5} fill={theme.halo} fillOpacity={0.88} />
          <path d={`M0 -6V0H${bar.px}V-6`} fill="none" stroke={theme.text} strokeWidth={1.5} />
          <text x={bar.px / 2} y={-9} fontSize={12} textAnchor="middle" fill={theme.text}>
            {formatKm(bar.km)}
          </text>
        </g>
      )}
    </svg>
  )
}

// ---------- Ekran okuyucu / tablo alternatifi ----------

interface MapTablesProps {
  scene: MapScene
  model: MapModel
  format: CoordFormat
  visible: boolean
  onSelect: (sel: Selection) => void
}

/** Harita verisinin tablo karşılığı: görünmezken de ekran okuyucuya açıktır (sr-only); görünürken satırlar seçilebilir. */
function MapTables({ scene, model, format, visible, onSelect }: MapTablesProps) {
  const unit = model.unit
  const name = (label: string, sel: Selection) =>
    visible ? (
      <button type="button" className="g19-row-btn" onClick={() => onSelect(sel)}>
        {label}
      </button>
    ) : (
      label
    )
  const regions = scene.countries
    .map((c, index) => ({ c, index }))
    .filter(({ c }) => c.value !== undefined)
    .sort((a, b) => (b.c.value ?? 0) - (a.c.value ?? 0))
  const more = (n: number) =>
    n > MAX_TABLE_ROWS ? (
      <tfoot>
        <tr>
          <td colSpan={4} className="g19-muted">
            İlk {MAX_TABLE_ROWS} satır gösteriliyor ({n} toplam).
          </td>
        </tr>
      </tfoot>
    ) : null

  return (
    <div className={visible ? 'g19-tables' : 'sr-only'}>
      {regions.length > 0 && (
        <table>
          <caption>Bölge değerleri</caption>
          <thead>
            <tr>
              <th scope="col">Bölge</th>
              <th scope="col">Değer</th>
            </tr>
          </thead>
          <tbody>
            {regions.slice(0, MAX_TABLE_ROWS).map(({ c, index }) => (
              <tr key={c.id}>
                <td>{name(c.name, { kind: 'country', index })}</td>
                <td className="g19-num">{formatValue(c.value ?? 0, unit)}</td>
              </tr>
            ))}
          </tbody>
          {more(regions.length)}
        </table>
      )}
      {(
        [
          ['İşaretçiler', scene.markers, true],
          ['Isı noktaları', scene.heat, false],
        ] as const
      ).map(([caption, list, selectable]) =>
        list.length > 0 ? (
          <table key={caption}>
            <caption>{caption}</caption>
            <thead>
              <tr>
                <th scope="col">Ad</th>
                <th scope="col">Koordinat (enlem, boylam)</th>
                <th scope="col">{selectable ? 'Değer' : 'Ağırlık'}</th>
              </tr>
            </thead>
            <tbody>
              {list.slice(0, MAX_TABLE_ROWS).map((m, index) => (
                <tr key={index}>
                  <td>{selectable ? name(m.label ?? `#${index + 1}`, { kind: 'marker', index }) : (m.label ?? `#${index + 1}`)}</td>
                  <td className="g19-coord">{formatCoord(m.lat, m.lon, format)}</td>
                  <td className="g19-num">{m.value !== undefined ? formatValue(m.value, selectable ? unit : undefined) : '–'}</td>
                </tr>
              ))}
            </tbody>
            {more(list.length)}
          </table>
        ) : null,
      )}
      {scene.routes.length > 0 && (
        <table>
          <caption>Rotalar</caption>
          <thead>
            <tr>
              <th scope="col">Rota</th>
              <th scope="col">Nokta</th>
              <th scope="col">Uzunluk</th>
              <th scope="col">Başlangıç → bitiş</th>
            </tr>
          </thead>
          <tbody>
            {scene.routes.slice(0, MAX_TABLE_ROWS).map((r, index) => {
              const a = r.coords[0]
              const b = r.coords[r.coords.length - 1]
              return (
                <tr key={index}>
                  <td>{name(r.label ?? `Rota ${index + 1}`, { kind: 'route', index })}</td>
                  <td className="g19-num">{r.coords.length}</td>
                  <td className="g19-num">{formatKm(r.km)}</td>
                  <td className="g19-coord">{a && b ? `${formatCoord(a[1], a[0], format)} → ${formatCoord(b[1], b[0], format)}` : ''}</td>
                </tr>
              )
            })}
          </tbody>
          {more(scene.routes.length)}
        </table>
      )}
      {scene.polygons.length > 0 && (
        <table>
          <caption>Çokgenler</caption>
          <thead>
            <tr>
              <th scope="col">Çokgen</th>
              <th scope="col">Köşe</th>
              <th scope="col">Değer</th>
            </tr>
          </thead>
          <tbody>
            {scene.polygons.slice(0, MAX_TABLE_ROWS).map((p, index) => (
              <tr key={index}>
                <td>{name(p.label ?? `Çokgen ${index + 1}`, { kind: 'polygon', index })}</td>
                <td className="g19-num">{p.rings[0]?.length ?? 0}</td>
                <td className="g19-num">{p.value !== undefined ? formatValue(p.value, unit) : '–'}</td>
              </tr>
            ))}
          </tbody>
          {more(scene.polygons.length)}
        </table>
      )}
    </div>
  )
}

registerRichBlock('map', G19MapBlock)
registerRichBlock('geojson', G19MapBlock)
