// [g18] ```chart ve ```sparkline blokları: grafik kartı (tür değiştirme, lejantla seri gizleme, yakınlaştırma,
//       grafik→tablo, CSV kopyalama, g17 dışa aktarma menüsü, istatistik özeti) ve satır içi mini grafik.
//       Erişilebilirlik: SVG aria etiketi + her zaman DOM'da duran veri tablosu (görünür değilse ekran okuyucuya).
import { ChartColumn, RotateCcw, Table2, ZoomIn, ZoomOut } from 'lucide-react'
import { useLayoutEffect, useMemo, useRef, useState, type RefObject } from 'react'
import { CopyButton } from './CopyButton.tsx'
import { G17DiagramExport } from './G17Blocks.tsx'
import { ChartView, kpiValues, legendFor, useChartTheme, valueFormatter, ZOOMABLE } from './G18Chart.tsx'
import { availableTypes, CHART_TYPE_LABELS, parseChart, specToTable, type ChartSpec, type ChartType } from './g18-chart-data.ts'
import { finite, formatNumber, FULL_WINDOW, summarize, zoomWindow } from './g18-chart-math.ts'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { toCsv } from './rich-data.ts'
import { BlockFrame, RawFallback } from './rich-shared.tsx'

/** Görünür/ekran okuyucu tablosunda en fazla satır (DOM boyutu sınırı). */
const TABLE_LIMIT = 500

function useWidth(): [RefObject<HTMLDivElement | null>, number] {
  const ref = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(640)
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const update = () => setWidth(Math.max(120, Math.floor(el.getBoundingClientRect().width)))
    update()
    const ro = new ResizeObserver(update)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])
  return [ref, width]
}

/** svg aria-label: başlık + tür + veri boyutu (ekran okuyucu ilk bunu duyar, ayrıntı tablodadır). */
function chartLabel(spec: ChartSpec, type: ChartType, fmt: (v: number | null) => string): string {
  const title = spec.title ?? 'Grafik'
  if (type === 'kpi') {
    const k = kpiValues(spec)
    return `${title}: ${k.label} ${fmt(k.value)}${k.previous !== null ? `, önceki ${fmt(k.previous)}` : ''}`
  }
  const points = spec.series.reduce((n, s) => n + finite(s.values).length, 0)
  return `${title}: ${CHART_TYPE_LABELS[type]} grafiği, ${spec.series.length} seri, ${points} değer. Veri tablosu grafiğin altındadır.`
}

interface ChartCardProps {
  spec: ChartSpec
  /** Dışa aktarma menüsündeki "Grafik kaynağı" (blok metni) */
  source: string
}

export function ChartCard({ spec, source }: ChartCardProps) {
  const theme = useChartTheme()
  const [type, setType] = useState<ChartType>(spec.type)
  const [hidden, setHidden] = useState<ReadonlySet<string>>(() => new Set())
  const [zoom, setZoom] = useState(FULL_WINDOW)
  const [showTable, setShowTable] = useState(false)
  const hostRef = useRef<HTMLDivElement>(null)
  const [measureRef, width] = useWidth()
  const fmt = useMemo(() => valueFormatter(spec.unit), [spec.unit])
  const types = availableTypes(spec)
  const legend = legendFor(spec, type, theme)
  const zoomable = ZOOMABLE.has(type)
  const zoomed = zoom.start > 0 || zoom.end < 1
  const table = useMemo(() => specToTable(spec, type, fmt), [spec, type, fmt])
  const stats = type === 'kpi' ? [] : spec.series.flatMap((s, i) => (hidden.has(`s${i}`) ? [] : [{ name: s.name, st: summarize(s.values) }]))
  const label = chartLabel(spec, type, fmt)

  const toggle = (key: string) =>
    setHidden((prev) => {
      const next = new Set(prev)
      if (!next.delete(key)) next.add(key)
      return next
    })
  const changeType = (next: ChartType) => {
    setType(next)
    setZoom(FULL_WINDOW)
    setHidden(new Set())
  }

  const tools = (
    <>
      {types.length > 1 && (
        <select className="g18-type" value={type} onChange={(e) => changeType(e.target.value as ChartType)} aria-label="Grafik türü">
          {types.map((t) => (
            <option key={t} value={t}>
              {CHART_TYPE_LABELS[t]}
            </option>
          ))}
        </select>
      )}
      {zoomable && (
        <>
          <button type="button" className="action-btn" onClick={() => setZoom((z) => zoomWindow(z, 0.8))} aria-label="Yakınlaştır" title="Yakınlaştır (+ ya da Ctrl+tekerlek)">
            <ZoomIn size={15} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" onClick={() => setZoom((z) => zoomWindow(z, 1.25))} disabled={!zoomed} aria-label="Uzaklaştır" title="Uzaklaştır (-)">
            <ZoomOut size={15} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" onClick={() => setZoom(FULL_WINDOW)} disabled={!zoomed} aria-label="Yakınlaştırmayı sıfırla" title="Sıfırla (0)">
            <RotateCcw size={15} aria-hidden="true" />
          </button>
        </>
      )}
      <button
        type="button"
        className={`action-btn${showTable ? ' g18-on' : ''}`}
        onClick={() => setShowTable((s) => !s)}
        aria-pressed={showTable}
        aria-label="Veriyi tablo olarak göster"
        title="Tablo görünümü"
      >
        <Table2 size={15} aria-hidden="true" />
      </button>
      <CopyButton className="action-btn" label="Veriyi CSV olarak kopyala" getText={() => toCsv(specToTable(spec, type))} />
      <G17DiagramExport hostRef={hostRef} source={source} sourceLabel="Grafik" />
    </>
  )

  return (
    <BlockFrame icon={<ChartColumn size={14} aria-hidden="true" />} label={spec.title ?? 'Grafik'} tools={tools} className="g18-chart">
      {legend.length > 1 && (
        <div className="g18-legend" role="group" aria-label="Lejant: göstermek ya da gizlemek için tıklayın">
          {legend.map((e) => (
            <button key={e.key} type="button" className="g18-legend-item" aria-pressed={!hidden.has(e.key)} onClick={() => toggle(e.key)}>
              <span className="g18-swatch" style={{ background: e.color }} />
              {e.name}
            </button>
          ))}
        </div>
      )}
      <div ref={measureRef} className="g18-measure">
        <ChartView key={type} spec={spec} type={type} hidden={hidden} zoom={zoom} onZoom={setZoom} width={width} theme={theme} label={label} hostRef={hostRef} />
      </div>
      {stats.length > 0 && (
        <details className="g18-stats">
          <summary>İstatistik özeti</summary>
          <table>
            <thead>
              <tr>
                <th scope="col">Seri</th>
                <th scope="col">Adet</th>
                <th scope="col">Min</th>
                <th scope="col">Maks</th>
                <th scope="col">Ortalama</th>
                <th scope="col">Medyan</th>
                <th scope="col">Toplam</th>
              </tr>
            </thead>
            <tbody>
              {stats.map(({ name, st }, i) => (
                <tr key={i}>
                  <th scope="row">{name}</th>
                  <td>{st?.count ?? 0}</td>
                  <td>{fmt(st?.min)}</td>
                  <td>{fmt(st?.max)}</td>
                  <td>{fmt(st?.mean)}</td>
                  <td>{fmt(st?.median)}</td>
                  <td>{fmt(st?.sum)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      )}
      <div className={showTable ? 'g18-table' : 'sr-only'}>
        <table>
          <caption>{`${spec.title ?? 'Grafik'} verisi`}</caption>
          <thead>
            <tr>
              {table.headers.map((h, i) => (
                <th key={i} scope="col">
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {table.rows.slice(0, TABLE_LIMIT).map((row, r) => (
              <tr key={r}>
                {row.map((cell, c) => (c === 0 ? <th key={c} scope="row">{cell}</th> : <td key={c}>{cell}</td>))}
              </tr>
            ))}
          </tbody>
        </table>
        {table.rows.length > TABLE_LIMIT && <p className="g8-note">{`İlk ${TABLE_LIMIT} satır gösteriliyor (toplam ${table.rows.length}); tamamı için CSV kopyalayın.`}</p>}
      </div>
    </BlockFrame>
  )
}

function parse(code: string, forced?: ChartType): { ok: true; spec: ChartSpec } | { ok: false; error: string } {
  try {
    return { ok: true, spec: parseChart(code, forced) }
  } catch (err) {
    return { ok: false, error: err instanceof Error ? err.message : 'Grafik ayrıştırılamadı' }
  }
}

export function ChartBlock({ code }: RichBlockProps) {
  const parsed = useMemo(() => parse(code), [code])
  if (!parsed.ok) return <RawFallback code={code} note={`Grafik çizilemedi: ${parsed.error}`} />
  // Kaynak değişince (akış tamamlanınca) tür/gizleme/yakınlaştırma sıfırlansın diye anahtar kaynaktır.
  return <ChartCard key={code} spec={parsed.spec} source={code} />
}

/** Satır içi mini grafik: küçük çizgi + son değer; ayrıntı aria etiketinde ve üzerine gelince ipucunda. */
export function SparklineBlock({ code }: RichBlockProps) {
  const theme = useChartTheme()
  const parsed = useMemo(() => parse(code, 'sparkline'), [code])
  if (!parsed.ok) return <RawFallback code={code} note={`Mini grafik çizilemedi: ${parsed.error}`} />
  const { spec } = parsed
  const fmt = valueFormatter(spec.unit)
  const values = spec.series[0]?.values ?? []
  const st = summarize(values)
  const last = finite(values).at(-1) ?? null
  const summary = st ? `en düşük ${fmt(st.min)}, en yüksek ${fmt(st.max)}, ortalama ${fmt(st.mean)}, son ${fmt(last)}` : 'veri yok'
  const label = `${spec.title ?? 'Mini grafik'}: ${st?.count ?? 0} değer, ${summary}`
  return (
    <span className="g18-spark" title={label}>
      <ChartView spec={spec} type="sparkline" hidden={new Set()} zoom={FULL_WINDOW} onZoom={() => undefined} width={160} theme={theme} label={label} />
      {spec.title && <span className="g18-spark-title">{spec.title}</span>}
      <span className="g18-spark-last" aria-hidden="true">
        {last === null ? '–' : formatNumber(last)}
      </span>
    </span>
  )
}

registerRichBlock('chart', ChartBlock)
registerRichBlock('sparkline', SparklineBlock)
registerRichBlock('spark', SparklineBlock)
