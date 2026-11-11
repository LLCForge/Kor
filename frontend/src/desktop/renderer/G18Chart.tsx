// [g18] Bağımlılıksız SVG grafik çizicisi: tüm türler tek <svg> içine React ile çizilir (model verisi yalnızca metin
//       düğümü ve sayısal öznitelik olarak girer; ham HTML/innerHTML yok). Renkler tema değişkeni yerine somut hex
//       değerlerdir; böylece g17 dışa aktarma menüsünün serileştirdiği SVG/PNG uygulama dışında da aynı görünür.
//       Palet: renk körlüğü için doğrulanmış 8 kategorik ton (açık/koyu tema için ayrı adımlar), sıra sabittir.
import { useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react'
import type { ChartSeries, ChartSpec, ChartType, RefLine, RefStat } from './g18-chart-data.ts'
import {
  boxStats,
  finite,
  FULL_WINDOW,
  formatNumber,
  histogram,
  inkOn,
  niceTicks,
  panWindow,
  scaleLinear,
  seqColor,
  stackValues,
  summarize,
  waterfall,
  windowIndices,
  zoomWindow,
  type Ticks,
  type ZoomWindow,
} from './g18-chart-math.ts'
import { percentText } from './ui-locale.ts'

export interface ChartTheme {
  dark: boolean
  palette: readonly string[]
  surface: string
  ink: string
  ink2: string
  muted: string
  grid: string
  axis: string
  good: string
  bad: string
}

/** Kökteki bir renk değişkeninin çözülmüş değeri (SVG sunum özniteliği var() almaz). */
function cssColor(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || 'currentColor'
}

const LIGHT: ChartTheme = {
  dark: false,
  palette: ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#008300', '#6250d6', '#e34948'],
  surface: '#ffffff',
  ink: '#1a1a18',
  ink2: '#3d3a35',
  muted: '#5b5950',
  grid: '#ebe7de',
  axis: '#c3c2b7',
  good: '#006300',
  bad: '#9b2c1c',
}
const DARK: ChartTheme = {
  dark: true,
  palette: ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#008300', '#9085e9', '#e66767'],
  surface: '#1f1e1b',
  // Marka mürekkebi (theme.json ink.cream) CSS değişkeninden okunur; tema hex'i kopyalanmaz (§16.2 kural 1).
  get ink() {
    return cssColor('--kor-ink-cream')
  },
  ink2: '#cdc9be',
  muted: '#a3a098',
  grid: '#33312d',
  axis: '#4a4842',
  good: '#0ca30c',
  bad: '#f0a39a',
}
/** 8. seriden sonrası ve "Diğer" dilimi için nötr gri (ton döngüsü yapılmaz). */
export const OTHER_COLOR = '#898781'
const FONT = "'Segoe UI', system-ui, -apple-system, sans-serif"

const darkQuery = () => window.matchMedia('(prefers-color-scheme: dark)')
const subscribeTheme = (cb: () => void) => {
  const mq = darkQuery()
  mq.addEventListener('change', cb)
  return () => mq.removeEventListener('change', cb)
}

/** Uygulama teması (nativeTheme → prefers-color-scheme) değişince grafik renkleri de değişir. */
export function useChartTheme(): ChartTheme {
  return useSyncExternalStore(subscribeTheme, () => darkQuery().matches) ? DARK : LIGHT
}

const seriesColor = (theme: ChartTheme, s: ChartSeries | undefined, i: number) => s?.color ?? theme.palette[i] ?? OTHER_COLOR

/** Yakınlaştırma/kaydırma desteklenen türler (x ekseni sürekli ya da kategorik sıralı). */
export const ZOOMABLE: ReadonlySet<ChartType> = new Set<ChartType>(['line', 'bar', 'stacked', 'area', 'dual', 'waterfall', 'scatter', 'geo'])

export interface LegendEntry {
  key: string
  name: string
  color: string
}

const MAX_SLICES = 8

/** Pasta/halka dilimleri: 8'den fazla kategori varsa ilk 7'si + "Diğer" (renk döngüsü yerine katlama). */
function slices(spec: ChartSpec, theme: ChartTheme): (LegendEntry & { value: number; index: number })[] {
  const s = spec.series[0]
  if (!s) return []
  const all = spec.labels.map((label, i) => ({ key: `c${i}`, name: label, color: theme.palette[i] ?? OTHER_COLOR, value: Math.max(0, s.values[i] ?? 0), index: i }))
  if (all.length <= MAX_SLICES) return all
  const rest = all.slice(MAX_SLICES - 1).reduce((sum, e) => sum + e.value, 0)
  return [...all.slice(0, MAX_SLICES - 1), { key: 'cother', name: 'Diğer', color: OTHER_COLOR, value: rest, index: -1 }]
}

/** Lejant girişleri: pasta/halkada kategoriler, çok serili türlerde seriler; tek anlamlı türlerde yok. */
export function legendFor(spec: ChartSpec, type: ChartType, theme: ChartTheme): LegendEntry[] {
  if (type === 'pie' || type === 'donut') return slices(spec, theme).map(({ key, name, color }) => ({ key, name, color }))
  if (['funnel', 'heatmap', 'kpi', 'sparkline', 'waterfall', 'histogram'].includes(type) || spec.series.length < 2) return []
  return spec.series.map((s, i) => ({ key: `s${i}`, name: s.name, color: seriesColor(theme, s, i) }))
}

// ---------- Çizim modeli ----------

interface TipRow {
  name: string
  value: string
  color?: string
}
interface Target {
  x: number
  y: number
  title: string
  rows: TipRow[]
}
interface Box {
  x0: number
  x1: number
  y0: number
  y1: number
}
interface Drawn {
  width: number
  height: number
  marks: ReactNode
  targets: Target[]
  /** x: en yakın sütun, xy: en yakın nokta, angle: radar ekseni, mark: işaretin kendi olayı */
  mode: 'x' | 'xy' | 'angle' | 'mark'
  plot: Box
  center?: { cx: number; cy: number; n: number }
}
interface Ctx {
  spec: ChartSpec
  type: ChartType
  width: number
  theme: ChartTheme
  hidden: ReadonlySet<string>
  zoom: ZoomWindow
  active: number | null
  hover: (i: number | null) => void
  fmt: (v: number | null | undefined) => string
}
interface Vis {
  s: ChartSeries
  i: number
}

const CHAR_W = 6.6
const r1 = (v: number) => Math.round(v * 10) / 10
const clip = (s: string, max = 16) => (s.length > max ? `${s.slice(0, max - 1)}…` : s)
const visible = (ctx: Ctx): Vis[] => ctx.spec.series.map((s, i) => ({ s, i })).filter(({ i }) => !ctx.hidden.has(`s${i}`))
const markHover = (ctx: Ctx, k: number) => ({ onPointerEnter: () => ctx.hover(k) })

const REF_NAMES: Record<RefStat, string> = { mean: 'Ortalama', median: 'Medyan', min: 'Min', max: 'Maks' }

function refNumber(ref: RefLine, vis: readonly Vis[]): number | null {
  if (typeof ref.value === 'number') return ref.value
  const source = vis.find((v) => v.s.axis === ref.axis) ?? vis[0]
  const st = source ? summarize(source.s.values) : null
  return st ? st[ref.value] : null
}

function emptyDrawn(width: number, theme: ChartTheme, text: string): Drawn {
  const height = 120
  return {
    width,
    height,
    mode: 'mark',
    targets: [],
    plot: { x0: 0, x1: width, y0: 0, y1: height },
    marks: (
      <text x={width / 2} y={height / 2} textAnchor="middle" fill={theme.muted} fontSize={13}>
        {text}
      </text>
    ),
  }
}

function yAxisMarks(box: Box, ticks: Ticks, ys: (v: number) => number, theme: ChartTheme, side: 'left' | 'right'): ReactNode {
  return ticks.ticks.map((t) => (
    <g key={`${side}${t}`}>
      {side === 'left' && <line x1={box.x0} x2={box.x1} y1={r1(ys(t))} y2={r1(ys(t))} stroke={theme.grid} />}
      <text x={side === 'left' ? box.x0 - 6 : box.x1 + 6} y={r1(ys(t))} dy="0.32em" textAnchor={side === 'left' ? 'end' : 'start'} fill={theme.muted} fontSize={11}>
        {formatNumber(t, true)}
      </text>
    </g>
  ))
}

function bandLabels(labels: readonly string[], cx: (j: number) => number, box: Box, theme: ChartTheme): ReactNode {
  const avg = labels.reduce((n, l) => n + Math.min(16, l.length), 0) / Math.max(1, labels.length)
  const step = Math.max(1, Math.ceil((labels.length * (avg * CHAR_W + 10)) / Math.max(1, box.x1 - box.x0)))
  return labels.map((l, j) =>
    j % step === 0 ? (
      <text key={j} x={r1(cx(j))} y={box.y1 + 16} textAnchor="middle" fill={theme.muted} fontSize={11}>
        {clip(l)}
      </text>
    ) : null,
  )
}

function axisTitles(box: Box, width: number, height: number, ctx: Ctx, right: boolean): ReactNode {
  const { spec, theme } = ctx
  const mid = (box.y0 + box.y1) / 2
  return (
    <>
      {spec.yLabel && (
        <text transform={`translate(12 ${mid}) rotate(-90)`} textAnchor="middle" fill={theme.ink2} fontSize={11}>
          {clip(spec.yLabel, 40)}
        </text>
      )}
      {right && spec.y2Label && (
        <text transform={`translate(${width - 10} ${mid}) rotate(90)`} textAnchor="middle" fill={theme.ink2} fontSize={11}>
          {clip(spec.y2Label, 40)}
        </text>
      )}
      {spec.xLabel && ctx.type !== 'geo' && (
        <text x={(box.x0 + box.x1) / 2} y={height - 4} textAnchor="middle" fill={theme.ink2} fontSize={11}>
          {clip(spec.xLabel, 60)}
        </text>
      )}
    </>
  )
}

function refMarks(ctx: Ctx, vis: readonly Vis[], box: Box, ys: (v: number) => number, ys2?: (v: number) => number): ReactNode {
  return ctx.spec.refLines.map((ref, k) => {
    const v = refNumber(ref, vis)
    if (v === null) return null
    const y = (ref.axis === 'right' && ys2 ? ys2 : ys)(v)
    if (!Number.isFinite(y) || y < box.y0 - 0.5 || y > box.y1 + 0.5) return null
    const name = ref.label ?? (typeof ref.value === 'string' ? REF_NAMES[ref.value] : 'Referans')
    return (
      <g key={`ref${k}`}>
        <line x1={box.x0} x2={box.x1} y1={r1(y)} y2={r1(y)} stroke={ctx.theme.ink2} strokeWidth={1.25} strokeDasharray="5 4" />
        <text x={box.x1 - 4} y={r1(y) - 5} textAnchor="end" fill={ctx.theme.ink2} fontSize={11} stroke={ctx.theme.surface} strokeWidth={3} paintOrder="stroke">
          {`${clip(name, 30)}: ${ctx.fmt(v)}`}
        </text>
      </g>
    )
  })
}

function refDomain(ctx: Ctx, vis: readonly Vis[], axis: 'left' | 'right'): number[] {
  return ctx.spec.refLines.filter((r) => r.axis === axis).flatMap((r) => refNumber(r, vis) ?? [])
}

function linePath(points: readonly ([number, number] | null)[]): string {
  let d = ''
  let pen = false
  for (const p of points) {
    if (!p) {
      pen = false
      continue
    }
    d += `${pen ? 'L' : 'M'}${r1(p[0])},${r1(p[1])}`
    pen = true
  }
  return d
}

/** Veri ucu (tabandan uzak kenar) yuvarlatılmış çubuk yolu. */
function barPath(x: number, w: number, yBase: number, yVal: number, radius = 4): string {
  const h = Math.abs(yBase - yVal)
  const r = Math.min(radius, w / 2, h)
  const [a, b, c, d] = [r1(x), r1(x + w), r1(yBase), r1(yVal)]
  if (r < 1) return `M${a},${c}H${b}V${d}H${a}Z`
  const s = yVal <= yBase ? 1 : -1
  return `M${a},${c}V${r1(yVal + s * r)}Q${a},${d} ${r1(x + r)},${d}H${r1(x + w - r)}Q${b},${d} ${b},${r1(yVal + s * r)}V${c}Z`
}

const extent = (xs: readonly number[]): [number, number] => (xs.length ? [Math.min(...xs), Math.max(...xs)] : [0, 1])
const tickWidth = (t: Ticks) => Math.max(...t.ticks.map((v) => formatNumber(v, true).length)) * CHAR_W + 10

// ---------- Kartezyen kategorik türler (çizgi, sütun, yığılmış, alan, çift eksen, şelale, histogram) ----------

type CartKind = 'line' | 'bar' | 'stacked' | 'area' | 'dual' | 'waterfall' | 'histogram'

function drawCartesian(ctx: Ctx, labels: readonly string[], vis: readonly Vis[], kind: CartKind): Drawn {
  const { theme, width, spec, fmt, active } = ctx
  const height = 280
  const steps = kind === 'waterfall' && vis[0] ? waterfall(labels, vis[0].s.values) : []
  const allLabels = kind === 'waterfall' ? steps.map((s) => s.label) : labels
  const [i0, i1] = windowIndices(allLabels.length, ctx.zoom)
  const count = Math.max(1, i1 - i0)
  const anyRight = vis.some((v) => v.s.axis === 'right')
  const axisOf = (v: Vis, pos: number): 'left' | 'right' => (kind === 'dual' && (anyRight ? v.s.axis === 'right' : pos > 0) ? 'right' : 'left')
  const kindOf = (v: Vis, pos: number): 'line' | 'bar' | 'area' =>
    kind === 'dual' ? (v.s.kind ?? (axisOf(v, pos) === 'left' ? 'bar' : 'line')) : kind === 'line' ? 'line' : kind === 'area' ? 'area' : 'bar'
  const stacked = kind === 'stacked' || (kind === 'area' && spec.stacked)
  const stacks = stacked ? stackValues(vis.map((v) => v.s.values)) : []

  const left: number[] = refDomain(ctx, vis, 'left')
  const right: number[] = kind === 'dual' ? refDomain(ctx, vis, 'right') : []
  for (let j = i0; j < i1; j++) {
    if (kind === 'waterfall') {
      const st = steps[j]
      if (st) left.push(st.start, st.end)
      continue
    }
    vis.forEach((v, pos) => {
      if (stacked) {
        const seg = stacks[pos]?.[j]
        if (seg) left.push(seg.y0, seg.y1)
        return
      }
      const val = v.s.values[j]
      if (val !== null && val !== undefined) (axisOf(v, pos) === 'right' ? right : left).push(val)
    })
  }
  if (kind !== 'line' && !(kind === 'dual' && vis.every((v, p) => axisOf(v, p) === 'right' || kindOf(v, p) === 'line'))) left.push(0)
  if (kind === 'dual' && vis.some((v, p) => axisOf(v, p) === 'right' && kindOf(v, p) === 'bar')) right.push(0)
  const lt = niceTicks(...extent(left), 5)
  const rt = right.length ? niceTicks(...extent(right), 5) : null

  const box: Box = {
    x0: tickWidth(lt) + (spec.yLabel ? 16 : 0),
    x1: width - (rt ? tickWidth(rt) + (spec.y2Label ? 16 : 0) : 14),
    y0: 12,
    y1: height - 26 - (spec.xLabel ? 16 : 0),
  }
  const ys = scaleLinear(lt.min, lt.max, box.y1, box.y0)
  const ys2 = rt ? scaleLinear(rt.min, rt.max, box.y1, box.y0) : undefined
  const band = (box.x1 - box.x0) / count
  const cx = (j: number) => box.x0 + (j - i0 + 0.5) * band
  const window = allLabels.slice(i0, i1)
  const color = (v: Vis) => seriesColor(theme, v.s, v.i)

  const marks: ReactNode[] = []
  if (active !== null && active < count) {
    marks.push(<rect key="band" x={r1(box.x0 + active * band)} y={box.y0} width={r1(band)} height={box.y1 - box.y0} fill={theme.ink} fillOpacity={0.06} />)
  }

  // Sütunlar (gruplu ya da yığılmış) ve şelale
  const barVis = vis.map((v, pos) => ({ v, pos })).filter(({ v, pos }) => kindOf(v, pos) === 'bar')
  const groupW = band * (kind === 'histogram' ? 1 : 0.72)
  const groups = stacked || kind === 'waterfall' ? 1 : Math.max(1, barVis.length)
  const slot = groupW / groups
  const gap = Math.min(2, slot * 0.2)
  const barW = Math.max(1, slot - gap)
  for (let j = i0; j < i1; j++) {
    const gx = cx(j) - groupW / 2 + gap / 2
    if (kind === 'waterfall') {
      const st = steps[j]
      if (!st) continue
      const fill = st.total ? OTHER_COLOR : st.delta >= 0 ? theme.palette[0]! : theme.palette[7]!
      const [top, base] = st.end >= st.start ? [st.end, st.start] : [st.start, st.end]
      marks.push(<rect key={`w${j}`} className="g18-anim-bar" x={r1(gx)} y={r1(ys(top))} width={r1(barW)} height={Math.max(1, r1(ys(base) - ys(top)))} rx={2} fill={fill} />)
      const next = steps[j + 1]
      if (next && j + 1 < i1 && !next.total) {
        marks.push(<line key={`wc${j}`} x1={r1(gx + barW)} x2={r1(cx(j + 1) - groupW / 2 + gap / 2)} y1={r1(ys(st.end))} y2={r1(ys(st.end))} stroke={theme.axis} strokeDasharray="3 3" />)
      }
      continue
    }
    barVis.forEach(({ v, pos }, g) => {
      const scale = axisOf(v, pos) === 'right' && ys2 ? ys2 : ys
      if (stacked) {
        const seg = stacks[pos]?.[j]
        if (!seg || seg.y0 === seg.y1) return
        const [top, base] = seg.y1 >= seg.y0 ? [seg.y1, seg.y0] : [seg.y0, seg.y1]
        marks.push(<rect key={`b${v.i}-${j}`} className="g18-anim-bar" x={r1(gx)} y={r1(scale(top))} width={r1(barW)} height={r1(scale(base) - scale(top))} fill={color(v)} stroke={theme.surface} strokeWidth={1} />)
        return
      }
      const val = v.s.values[j]
      if (val === null || val === undefined) return
      const base = scale(Math.min(Math.max(0, lt.min), lt.max))
      marks.push(<path key={`b${v.i}-${j}`} className="g18-anim-bar" d={barPath(gx + g * slot, barW, base, scale(val))} fill={color(v)} />)
    })
  }

  // Çizgi ve alan serileri
  vis.forEach((v, pos) => {
    const k = kindOf(v, pos)
    if (k === 'bar') return
    const scale = axisOf(v, pos) === 'right' && ys2 ? ys2 : ys
    const pts: ([number, number] | null)[] = []
    for (let j = i0; j < i1; j++) {
      const val = stacked ? (stacks[pos]?.[j]?.y1 ?? null) : v.s.values[j]
      pts.push(val === null || val === undefined ? null : [cx(j), scale(val)])
    }
    if (k === 'area') {
      const baseOf = (j: number) => (stacked ? scale(stacks[pos]?.[j]?.y0 ?? 0) : scale(Math.min(Math.max(0, lt.min), lt.max)))
      let d = ''
      let run: number[] = []
      const flush = () => {
        if (run.length) {
          const top = run.map((j) => `${r1(cx(j))},${r1(pts[j - i0]![1])}`).join('L')
          const bottom = [...run].reverse().map((j) => `${r1(cx(j))},${r1(baseOf(j))}`).join('L')
          d += `M${top}L${bottom}Z`
        }
        run = []
      }
      for (let j = i0; j < i1; j++) (pts[j - i0] ? run.push(j) : flush())
      flush()
      marks.push(<path key={`a${v.i}`} className="g18-anim-fade" d={d} fill={color(v)} fillOpacity={stacked ? 0.55 : 0.18} />)
    }
    marks.push(<path key={`l${v.i}`} className="g18-anim-line" pathLength={1} d={linePath(pts)} fill="none" stroke={color(v)} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />)
    if (count <= 60) {
      for (const [j, p] of pts.entries()) {
        if (p) marks.push(<circle key={`p${v.i}-${j}`} cx={r1(p[0])} cy={r1(p[1])} r={j === active ? 5 : 3} fill={color(v)} stroke={theme.surface} strokeWidth={1.5} />)
      }
    } else if (active !== null && pts[active]) {
      const p = pts[active]!
      marks.push(<circle key={`pa${v.i}`} cx={r1(p[0])} cy={r1(p[1])} r={5} fill={color(v)} stroke={theme.surface} strokeWidth={2} />)
    }
  })

  const zeroY = lt.min < 0 && lt.max > 0 ? ys(0) : null
  const targets: Target[] = window.map((label, k) => {
    const j = i0 + k
    if (kind === 'waterfall') {
      const st = steps[j]!
      return {
        x: cx(j),
        y: ys(Math.max(st.start, st.end)),
        title: label,
        rows: st.total ? [{ name: 'Toplam', value: fmt(st.end) }] : [{ name: 'Değişim', value: `${st.delta > 0 ? '+' : ''}${fmt(st.delta)}` }, { name: 'Kümülatif', value: fmt(st.end) }],
      }
    }
    const tops = vis.map((v, pos) => {
      const val = stacked ? stacks[pos]?.[j]?.y1 : v.s.values[j]
      return val === null || val === undefined ? box.y1 : (axisOf(v, pos) === 'right' && ys2 ? ys2 : ys)(val)
    })
    return {
      x: cx(j),
      y: Math.min(box.y1, ...tops),
      title: label,
      rows: vis.map((v) => ({ name: v.s.name, value: fmt(v.s.values[j]), color: color(v) })),
    }
  })

  return {
    width,
    height,
    mode: 'x',
    plot: box,
    targets,
    marks: (
      <>
        {yAxisMarks(box, lt, ys, theme, 'left')}
        {rt && ys2 && yAxisMarks(box, rt, ys2, theme, 'right')}
        <line x1={box.x0} x2={box.x1} y1={r1(zeroY ?? box.y1)} y2={r1(zeroY ?? box.y1)} stroke={theme.axis} />
        {bandLabels(window, (k) => cx(i0 + k), box, theme)}
        {axisTitles(box, width, height, ctx, !!rt)}
        {marks}
        {refMarks(ctx, vis, box, ys, ys2)}
      </>
    ),
  }
}

function drawHistogram(ctx: Ctx): Drawn {
  const src = visible(ctx)[0]
  if (!src) return emptyDrawn(ctx.width, ctx.theme, 'Gösterilecek seri yok')
  const bins = histogram(src.s.values, ctx.spec.bins)
  const labels = bins.map((b) => `${formatNumber(b.x0, true)}–${formatNumber(b.x1, true)}`)
  const derived: Vis = { s: { name: 'Adet', values: bins.map((b) => b.count), axis: 'left' }, i: src.i }
  return drawCartesian({ ...ctx, fmt: (v) => formatNumber(v), spec: { ...ctx.spec, refLines: [], yLabel: 'Adet', xLabel: src.s.name } }, labels, [derived], 'histogram')
}

// ---------- Dağılım ve coğrafi nokta ----------

const geoLabel = (v: number, pos: string, neg: string) => (v === 0 ? '0°' : `${Math.abs(v)}°${v > 0 ? pos : neg}`)

function drawScatter(ctx: Ctx): Drawn {
  const { theme, width, fmt, spec, active } = ctx
  const geo = ctx.type === 'geo'
  const vis = visible(ctx)
  const all = vis.flatMap((v) =>
    v.s.values.flatMap((y, k) => {
      const x = v.s.xs ? v.s.xs[k] : k + 1
      return x === null || x === undefined || y === null ? [] : [{ v, k, x, y }]
    }),
  )
  if (all.length === 0) return emptyDrawn(width, theme, 'Gösterilecek nokta yok')
  const xt = geo ? { min: -180, max: 180 } : niceTicks(...extent(all.map((p) => p.x)), 6)
  const span = xt.max - xt.min
  const d0 = xt.min + ctx.zoom.start * span
  const d1 = xt.min + ctx.zoom.end * span
  const yt = geo ? { min: -90, max: 90, step: 30, ticks: [-90, -60, -30, 0, 30, 60, 90] } : niceTicks(...extent([...all.map((p) => p.y), ...refDomain(ctx, vis, 'left')]), 5)
  const left = (geo ? 44 : tickWidth(yt)) + (spec.yLabel && !geo ? 16 : 0)
  const plotW = width - left - 16
  const height = geo ? Math.round(Math.min(440, Math.max(200, plotW / 2 + 40))) : 300
  const box: Box = { x0: left, x1: width - 16, y0: 12, y1: height - 26 - (spec.xLabel && !geo ? 16 : 0) }
  const xs = scaleLinear(d0, d1, box.x0, box.x1)
  const ys = scaleLinear(yt.min, yt.max, box.y1, box.y0)
  const xTicks = geo
    ? Array.from({ length: 13 }, (_, i) => -180 + i * 30).filter((t) => t >= d0 && t <= d1)
    : niceTicks(d0, d1, 6).ticks.filter((t) => t >= d0 - 1e-9 && t <= d1 + 1e-9)
  const inView = all.filter((p) => p.x >= d0 && p.x <= d1)
  const ring = inView.length <= 1500
  const targets: Target[] = inView.map((p) => ({
    x: xs(p.x),
    y: ys(p.y),
    title: p.v.s.pointLabels?.[p.k] || p.v.s.name,
    rows: [
      ...(vis.length > 1 || p.v.s.pointLabels?.[p.k] ? [{ name: 'Seri', value: p.v.s.name, color: seriesColor(theme, p.v.s, p.v.i) }] : []),
      { name: geo ? 'Enlem' : (spec.yLabel ?? 'Y'), value: geo ? `${formatNumber(p.y)}°` : fmt(p.y) },
      { name: geo ? 'Boylam' : (spec.xLabel ?? 'X'), value: geo ? `${formatNumber(p.x)}°` : formatNumber(p.x) },
    ],
  }))
  const act = active !== null ? inView[active] : undefined
  return {
    width,
    height,
    mode: 'xy',
    plot: box,
    targets,
    marks: (
      <>
        {geo ? (
          <>
            {yt.ticks.map((t) => (
              <g key={`gy${t}`}>
                <line x1={box.x0} x2={box.x1} y1={r1(ys(t))} y2={r1(ys(t))} stroke={t === 0 ? theme.axis : theme.grid} />
                <text x={box.x0 - 6} y={r1(ys(t))} dy="0.32em" textAnchor="end" fill={theme.muted} fontSize={11}>
                  {geoLabel(t, 'K', 'G')}
                </text>
              </g>
            ))}
          </>
        ) : (
          yAxisMarks(box, yt as Ticks, ys, theme, 'left')
        )}
        {xTicks.map((t) => (
          <g key={`gx${t}`}>
            <line x1={r1(xs(t))} x2={r1(xs(t))} y1={box.y0} y2={box.y1} stroke={geo && t === 0 ? theme.axis : theme.grid} />
            <text x={r1(xs(t))} y={box.y1 + 16} textAnchor="middle" fill={theme.muted} fontSize={11}>
              {geo ? geoLabel(t, 'D', 'B') : formatNumber(t, true)}
            </text>
          </g>
        ))}
        <rect x={box.x0} y={box.y0} width={box.x1 - box.x0} height={box.y1 - box.y0} fill="none" stroke={theme.axis} />
        {axisTitles(box, width, height, ctx, false)}
        {inView.map((p) => (
          <circle
            key={`${p.v.i}-${p.k}`}
            className="g18-anim-fade"
            cx={r1(xs(p.x))}
            cy={r1(ys(p.y))}
            r={4}
            fill={seriesColor(theme, p.v.s, p.v.i)}
            fillOpacity={0.85}
            stroke={ring ? theme.surface : 'none'}
            strokeWidth={1}
          />
        ))}
        {act && <circle cx={r1(xs(act.x))} cy={r1(ys(act.y))} r={7} fill="none" stroke={theme.ink} strokeWidth={2} />}
        {!geo && refMarks(ctx, vis, box, ys)}
      </>
    ),
  }
}

// ---------- Pasta / halka ----------

function arcPath(cx: number, cy: number, r: number, inner: number, a0: number, a1: number): string {
  const pt = (rad: number, a: number) => `${r1(cx + rad * Math.sin(a))},${r1(cy - rad * Math.cos(a))}`
  if (a1 - a0 >= Math.PI * 2 - 1e-6) {
    // Tam daire: iki yarım yay (tek yay başlangıç = bitiş olduğu için çizilmez).
    const ring = (rad: number) => `M${pt(rad, 0)}A${rad},${rad} 0 1 1 ${pt(rad, Math.PI)}A${rad},${rad} 0 1 1 ${pt(rad, 0)}Z`
    return inner > 0 ? `${ring(r)}${ring(inner)}` : ring(r)
  }
  const large = a1 - a0 > Math.PI ? 1 : 0
  const outer = `M${pt(r, a0)}A${r},${r} 0 ${large} 1 ${pt(r, a1)}`
  return inner > 0 ? `${outer}L${pt(inner, a1)}A${inner},${inner} 0 ${large} 0 ${pt(inner, a0)}Z` : `${outer}L${r1(cx)},${r1(cy)}Z`
}

function drawPie(ctx: Ctx): Drawn {
  const { theme, width, fmt, active } = ctx
  const entries = slices(ctx.spec, theme).filter((e) => !ctx.hidden.has(e.key) && e.value > 0)
  const total = entries.reduce((s, e) => s + e.value, 0)
  if (total <= 0) return emptyDrawn(width, theme, 'Gösterilecek pozitif değer yok')
  const height = 280
  const cx = width / 2
  const cy = height / 2
  const r = Math.max(40, Math.min(height / 2 - 30, width / 2 - 130))
  const inner = ctx.type === 'donut' ? r * 0.6 : 0
  let a = 0
  const parts = entries.map((e) => {
    const a0 = a
    a += (e.value / total) * Math.PI * 2
    return { e, a0, a1: a, mid: (a0 + a) / 2, share: e.value / total }
  })
  const pct = (v: number) => percentText(formatNumber(v * 100))
  return {
    width,
    height,
    mode: 'mark',
    plot: { x0: 0, x1: width, y0: 0, y1: height },
    targets: parts.map(({ e, mid, share }) => {
      const rr = (r + inner) / 2 || r / 2
      return { x: cx + rr * Math.sin(mid), y: cy - rr * Math.cos(mid), title: e.name, rows: [{ name: 'Değer', value: fmt(e.value), color: e.color }, { name: 'Pay', value: pct(share) }] }
    }),
    marks: (
      <>
        {parts.map(({ e, a0, a1, mid }, k) => {
          const off = k === active ? 5 : 0
          return (
            <path
              key={e.key}
              className="g18-anim-fade"
              d={arcPath(cx + off * Math.sin(mid), cy - off * Math.cos(mid), r, inner, a0, a1)}
              fill={e.color}
              stroke={theme.surface}
              strokeWidth={2}
              fillRule="evenodd"
              {...markHover(ctx, k)}
            />
          )
        })}
        {parts.map(({ e, mid, share }) => {
          if (share < 0.03) return null
          const lx = cx + (r + 12) * Math.sin(mid)
          const ly = cy - (r + 12) * Math.cos(mid)
          return (
            <text key={`t${e.key}`} x={r1(lx)} y={r1(ly)} dy="0.32em" textAnchor={Math.sin(mid) >= 0 ? 'start' : 'end'} fill={theme.ink2} fontSize={11}>
              {`${clip(e.name, 18)} ${pct(share)}`}
            </text>
          )
        })}
        {inner > 0 && (
          <>
            <text x={cx} y={cy - 4} textAnchor="middle" fill={theme.ink} fontSize={18} fontWeight={600}>
              {formatNumber(total, true)}
            </text>
            <text x={cx} y={cy + 14} textAnchor="middle" fill={theme.muted} fontSize={11}>
              Toplam
            </text>
          </>
        )}
      </>
    ),
  }
}

// ---------- Huni ----------

function drawFunnel(ctx: Ctx): Drawn {
  const { theme, width, fmt, spec, active } = ctx
  const src = visible(ctx)[0]
  if (!src) return emptyDrawn(width, theme, 'Gösterilecek seri yok')
  const stages = spec.labels.map((label, j) => ({ label, value: Math.max(0, src.s.values[j] ?? 0) }))
  const max = Math.max(1e-9, ...stages.map((s) => s.value))
  const first = stages[0]?.value || 1
  const labelW = Math.min(170, Math.max(...stages.map((s) => Math.min(24, s.label.length))) * CHAR_W + 16)
  const valueW = 130
  const area = Math.max(60, width - labelW - valueW)
  const rowH = 34
  const height = stages.length * (rowH + 6) + 8
  const n = Math.max(1, stages.length - 1)
  return {
    width,
    height,
    mode: 'mark',
    plot: { x0: labelW, x1: labelW + area, y0: 0, y1: height },
    targets: stages.map((s, j) => ({
      x: labelW + area / 2,
      y: 4 + j * (rowH + 6),
      title: s.label,
      rows: [
        { name: 'Değer', value: fmt(s.value) },
        { name: 'İlk aşamaya göre', value: percentText(formatNumber((s.value / first) * 100)) },
        ...(j > 0 ? [{ name: 'Önceki aşamaya göre', value: percentText(formatNumber((s.value / (stages[j - 1]!.value || 1)) * 100)) }] : []),
      ],
    })),
    marks: stages.map((s, j) => {
      const w = Math.max(2, (s.value / max) * area)
      const y = 4 + j * (rowH + 6)
      return (
        <g key={j} {...markHover(ctx, j)}>
          <rect x={0} y={y} width={width} height={rowH} fill={j === active ? theme.ink : 'transparent'} fillOpacity={0.05} />
          <rect className="g18-anim-bar" x={r1(labelW + (area - w) / 2)} y={y} width={r1(w)} height={rowH} rx={4} fill={seqColor(0.8 - (0.5 * j) / n, theme.dark)} />
          <text x={labelW - 8} y={y + rowH / 2} dy="0.32em" textAnchor="end" fill={theme.ink} fontSize={12}>
            {clip(s.label, 24)}
          </text>
          <text x={labelW + area + 8} y={y + rowH / 2} dy="0.32em" fill={theme.ink2} fontSize={12}>
            {`${fmt(s.value)} · %${formatNumber((s.value / first) * 100)}`}
          </text>
        </g>
      )
    }),
  }
}

// ---------- Isı haritası ----------

const MAX_HEAT_CELLS = 2500

function drawHeatmap(ctx: Ctx): Drawn {
  const { theme, width, fmt, spec, active } = ctx
  const rows = visible(ctx)
  if (rows.length === 0) return emptyDrawn(width, theme, 'Gösterilecek seri yok')
  const cols = spec.labels
  // Hücre başına 2-3 düğüm: okunaksız ve ağır matrisler çizilmez (tablo görünümü yine kullanılabilir).
  if (rows.length * cols.length > MAX_HEAT_CELLS) return emptyDrawn(width, theme, `Isı haritası en fazla ${MAX_HEAT_CELLS} hücre gösterebilir`)
  const [min, max] = extent(rows.flatMap((r) => finite(r.s.values)))
  const labelW = Math.min(150, Math.max(...rows.map((r) => Math.min(22, r.s.name.length))) * CHAR_W + 12)
  const cellW = (width - labelW - 8) / Math.max(1, cols.length)
  const cellH = Math.max(14, Math.min(36, 260 / rows.length))
  const top = 6
  const gridBottom = top + rows.length * cellH
  const height = gridBottom + 50
  const box: Box = { x0: labelW, x1: width - 8, y0: top, y1: gridBottom }
  const showValues = cellW >= 34 && cellH >= 18
  const t = (v: number) => (max === min ? 0.5 : (v - min) / (max - min))
  const targets: Target[] = []
  const cells: ReactNode[] = []
  rows.forEach((row, ri) => {
    cols.forEach((col, ci) => {
      const v = row.s.values[ci] ?? null
      const k = targets.length
      const x = labelW + ci * cellW
      const y = top + ri * cellH
      const fill = v === null ? theme.surface : seqColor(t(v), theme.dark)
      targets.push({ x: x + cellW / 2, y, title: `${row.s.name} · ${col}`, rows: [{ name: 'Değer', value: fmt(v) }] })
      cells.push(
        <g key={`${ri}-${ci}`} {...markHover(ctx, k)}>
          <rect className="g18-anim-fade" x={r1(x)} y={r1(y)} width={r1(Math.max(1, cellW - 1))} height={r1(cellH - 1)} fill={fill} stroke={k === active ? theme.ink : v === null ? theme.grid : 'none'} strokeWidth={k === active ? 2 : 1} />
          {showValues && v !== null && (
            <text x={r1(x + cellW / 2)} y={r1(y + cellH / 2)} dy="0.32em" textAnchor="middle" fontSize={10} fill={inkOn(fill) === 'dark' ? '#0b0b0b' : '#f0efec'}>
              {formatNumber(v, true)}
            </text>
          )}
        </g>,
      )
    })
  })
  const legendY = gridBottom + 30
  return {
    width,
    height,
    mode: 'mark',
    plot: box,
    targets,
    marks: (
      <>
        {rows.map((row, ri) => (
          <text key={`r${ri}`} x={labelW - 6} y={r1(top + ri * cellH + cellH / 2)} dy="0.32em" textAnchor="end" fill={theme.ink2} fontSize={11}>
            {clip(row.s.name, 22)}
          </text>
        ))}
        {bandLabels(cols, (ci) => labelW + ci * cellW + cellW / 2, box, theme)}
        {cells}
        <text x={labelW} y={legendY + 5} dy="0.32em" fill={theme.muted} fontSize={11}>
          {formatNumber(min, true)}
        </text>
        {[0, 0.25, 0.5, 0.75, 1].map((s, i) => (
          <rect key={`lg${s}`} x={labelW + 48 + i * 22} y={legendY} width={22} height={10} fill={seqColor(s, theme.dark)} />
        ))}
        <text x={labelW + 48 + 5 * 22 + 6} y={legendY + 5} dy="0.32em" fill={theme.muted} fontSize={11}>
          {formatNumber(max, true)}
        </text>
      </>
    ),
  }
}

// ---------- Kutu grafiği ----------

function drawBox(ctx: Ctx): Drawn {
  const { theme, width, fmt, active } = ctx
  const vis = visible(ctx)
    .map((v) => ({ v, b: boxStats(v.s.values) }))
    .filter((x): x is { v: Vis; b: NonNullable<ReturnType<typeof boxStats>> } => x.b !== null)
  if (vis.length === 0) return emptyDrawn(width, theme, 'Gösterilecek seri yok')
  const height = 280
  const yt = niceTicks(Math.min(...vis.map((x) => x.b.min)), Math.max(...vis.map((x) => x.b.max)), 5)
  const box: Box = { x0: tickWidth(yt) + (ctx.spec.yLabel ? 16 : 0), x1: width - 14, y0: 12, y1: height - 26 }
  const ys = scaleLinear(yt.min, yt.max, box.y1, box.y0)
  const band = (box.x1 - box.x0) / vis.length
  const bw = Math.min(64, band * 0.5)
  const cx = (k: number) => box.x0 + (k + 0.5) * band
  return {
    width,
    height,
    mode: 'x',
    plot: box,
    targets: vis.map(({ v, b }, k) => ({
      x: cx(k),
      y: ys(b.max),
      title: v.s.name,
      rows: [
        { name: 'Maks', value: fmt(b.max) },
        { name: '3. çeyrek', value: fmt(b.q3) },
        { name: 'Medyan', value: fmt(b.median) },
        { name: '1. çeyrek', value: fmt(b.q1) },
        { name: 'Min', value: fmt(b.min) },
        ...(b.outliers.length ? [{ name: 'Aykırı değer', value: String(b.outliers.length) }] : []),
      ],
    })),
    marks: (
      <>
        {yAxisMarks(box, yt, ys, theme, 'left')}
        {axisTitles(box, width, height, { ...ctx, spec: { ...ctx.spec, xLabel: undefined } }, false)}
        {bandLabels(
          vis.map(({ v }) => v.s.name),
          cx,
          box,
          theme,
        )}
        {vis.map(({ v, b }, k) => {
          const c = seriesColor(theme, v.s, v.i)
          const x = cx(k)
          return (
            <g key={v.i} className="g18-anim-fade">
              {k === active && <rect x={r1(x - band / 2)} y={box.y0} width={r1(band)} height={box.y1 - box.y0} fill={theme.ink} fillOpacity={0.06} />}
              <line x1={r1(x)} x2={r1(x)} y1={r1(ys(b.hi))} y2={r1(ys(b.q3))} stroke={theme.ink2} />
              <line x1={r1(x)} x2={r1(x)} y1={r1(ys(b.q1))} y2={r1(ys(b.lo))} stroke={theme.ink2} />
              <line x1={r1(x - bw / 4)} x2={r1(x + bw / 4)} y1={r1(ys(b.hi))} y2={r1(ys(b.hi))} stroke={theme.ink2} />
              <line x1={r1(x - bw / 4)} x2={r1(x + bw / 4)} y1={r1(ys(b.lo))} y2={r1(ys(b.lo))} stroke={theme.ink2} />
              <rect x={r1(x - bw / 2)} y={r1(ys(b.q3))} width={r1(bw)} height={Math.max(1, r1(ys(b.q1) - ys(b.q3)))} rx={3} fill={c} fillOpacity={0.3} stroke={c} strokeWidth={1.5} />
              <line x1={r1(x - bw / 2)} x2={r1(x + bw / 2)} y1={r1(ys(b.median))} y2={r1(ys(b.median))} stroke={theme.ink} strokeWidth={2} />
              {b.outliers.map((o, oi) => (
                <circle key={oi} cx={r1(x)} cy={r1(ys(o))} r={3} fill={theme.surface} stroke={c} strokeWidth={1.5} />
              ))}
            </g>
          )
        })}
        {refMarks(ctx, visible(ctx), box, ys)}
      </>
    ),
  }
}

// ---------- Radar ----------

function drawRadar(ctx: Ctx): Drawn {
  const { theme, width, fmt, spec, active } = ctx
  const vis = visible(ctx)
  const n = spec.labels.length
  if (vis.length === 0 || n < 3) return emptyDrawn(width, theme, n < 3 ? 'Radar için en az 3 eksen gerekir' : 'Gösterilecek seri yok')
  const height = 320
  const cx = width / 2
  const cy = height / 2
  const R = Math.max(40, Math.min(height / 2 - 34, width / 2 - 90))
  const t = niceTicks(Math.min(0, ...vis.flatMap((v) => finite(v.s.values))), Math.max(...vis.flatMap((v) => finite(v.s.values)), 0), 4)
  const rs = scaleLinear(t.min, t.max, 0, R)
  const ang = (i: number) => (i / n) * Math.PI * 2
  const at = (i: number, rad: number): [number, number] => [cx + rad * Math.sin(ang(i)), cy - rad * Math.cos(ang(i))]
  const poly = (rad: (i: number) => number) => spec.labels.map((_, i) => at(i, rad(i)).map(r1).join(',')).join(' ')
  return {
    width,
    height,
    mode: 'angle',
    center: { cx, cy, n },
    plot: { x0: cx - R, x1: cx + R, y0: cy - R, y1: cy + R },
    targets: spec.labels.map((label, i) => {
      const [x, y] = at(i, R)
      return { x, y, title: label, rows: vis.map((v) => ({ name: v.s.name, value: fmt(v.s.values[i]), color: seriesColor(theme, v.s, v.i) })) }
    }),
    marks: (
      <>
        {t.ticks
          .filter((v) => v > t.min)
          .map((v) => (
            <polygon key={`ring${v}`} points={poly(() => rs(v))} fill="none" stroke={theme.grid} />
          ))}
        {spec.labels.map((label, i) => {
          const [x, y] = at(i, R)
          const [lx, ly] = at(i, R + 14)
          const s = Math.sin(ang(i))
          return (
            <g key={`ax${i}`}>
              <line x1={cx} y1={cy} x2={r1(x)} y2={r1(y)} stroke={i === active ? theme.ink2 : theme.grid} />
              <text x={r1(lx)} y={r1(ly)} dy="0.32em" textAnchor={Math.abs(s) < 0.2 ? 'middle' : s > 0 ? 'start' : 'end'} fill={theme.ink2} fontSize={11}>
                {clip(label, 18)}
              </text>
            </g>
          )
        })}
        {/* En dış halka etiketi üstteki eksen adıyla çakışmasın diye yazılmaz (değer ipucunda). */}
        {t.ticks.slice(0, -1).map((v) => (
          <text key={`tl${v}`} x={cx + 3} y={r1(cy - rs(v))} dy="-0.2em" fill={theme.muted} fontSize={10}>
            {formatNumber(v, true)}
          </text>
        ))}
        {vis.map((v) => {
          const c = seriesColor(theme, v.s, v.i)
          return (
            <g key={v.i} className="g18-anim-fade">
              <polygon points={poly((i) => rs(v.s.values[i] ?? t.min))} fill={c} fillOpacity={0.14} stroke={c} strokeWidth={2} strokeLinejoin="round" />
              {spec.labels.map((_, i) => {
                const val = v.s.values[i]
                if (val === null || val === undefined) return null
                const [x, y] = at(i, rs(val))
                return <circle key={i} cx={r1(x)} cy={r1(y)} r={i === active ? 5 : 3} fill={c} stroke={theme.surface} strokeWidth={1.5} />
              })}
            </g>
          )
        })}
      </>
    ),
  }
}

// ---------- KPI kartı ve mini grafik ----------

function sparkPath(values: readonly (number | null)[], box: Box): { d: string; last: [number, number] | null } {
  const xsN = Math.max(1, values.length - 1)
  const [min, max] = extent(finite(values))
  const ys = scaleLinear(min, max, box.y1, box.y0)
  const pts = values.map((v, i): [number, number] | null => (v === null ? null : [box.x0 + (i / xsN) * (box.x1 - box.x0), ys(v)]))
  return { d: linePath(pts), last: pts.findLast((p) => p !== null) ?? null }
}

/** KPI değeri: açık değer ya da ilk serinin son değeri; önceki: açık değer ya da sondan ikinci değer. */
export function kpiValues(spec: ChartSpec): { value: number | null; previous: number | null; label: string } {
  const xs = finite(spec.series[0]?.values ?? [])
  return {
    value: spec.kpi?.value ?? xs.at(-1) ?? null,
    previous: spec.kpi?.previous ?? (spec.kpi?.value === undefined ? (xs.at(-2) ?? null) : null),
    label: spec.kpi?.label ?? spec.title ?? spec.series[0]?.name ?? 'Gösterge',
  }
}

function drawKpi(ctx: Ctx): Drawn {
  const { theme, width, fmt, spec } = ctx
  const { value, previous, label } = kpiValues(spec)
  const height = 118
  const delta = value !== null && previous !== null ? value - previous : null
  const better = delta === null || delta === 0 ? null : delta > 0 !== !!spec.kpi?.lowerIsBetter
  const deltaColor = better === null ? theme.muted : better ? theme.good : theme.bad
  const series = spec.series[0]?.values ?? []
  const showSpark = width >= 380 && finite(series).length >= 2
  const spark = showSpark ? sparkPath(series, { x0: width * 0.58, x1: width - 18, y0: 24, y1: height - 24 }) : null
  const pct = delta !== null && previous ? ` (%${formatNumber((delta / Math.abs(previous)) * 100)})` : ''
  return {
    width,
    height,
    mode: 'mark',
    targets: [],
    plot: { x0: 0, x1: width, y0: 0, y1: height },
    marks: (
      <>
        <text x={18} y={28} fill={theme.muted} fontSize={13}>
          {clip(label, 60)}
        </text>
        <text x={18} y={70} fill={theme.ink} fontSize={34} fontWeight={600}>
          {fmt(value)}
        </text>
        {delta !== null && (
          <>
            <path d={delta >= 0 ? 'M18,104 l6,-10 l6,10 Z' : 'M18,94 l6,10 l6,-10 Z'} fill={deltaColor} />
            <text x={36} y={103} fill={deltaColor} fontSize={13} fontWeight={600}>
              {`${delta > 0 ? '+' : ''}${formatNumber(delta)}${pct}`}
              <tspan fill={theme.muted} fontWeight={400}>
                {' önceki değere göre'}
              </tspan>
            </text>
          </>
        )}
        {spark && (
          <>
            <path className="g18-anim-line" pathLength={1} d={spark.d} fill="none" stroke={theme.palette[0]} strokeWidth={2} strokeLinejoin="round" />
            {spark.last && <circle cx={r1(spark.last[0])} cy={r1(spark.last[1])} r={3.5} fill={theme.palette[0]} stroke={theme.surface} strokeWidth={1.5} />}
          </>
        )}
      </>
    ),
  }
}

function drawSpark(ctx: Ctx): Drawn {
  const { theme } = ctx
  const width = Math.min(ctx.width, 240)
  const height = 44
  const values = visible(ctx)[0]?.s.values ?? []
  const spark = sparkPath(values, { x0: 4, x1: width - 6, y0: 6, y1: height - 6 })
  return {
    width,
    height,
    mode: 'mark',
    targets: [],
    plot: { x0: 0, x1: width, y0: 0, y1: height },
    marks: (
      <>
        <path className="g18-anim-line" pathLength={1} d={spark.d} fill="none" stroke={theme.palette[0]} strokeWidth={1.75} strokeLinejoin="round" strokeLinecap="round" />
        {spark.last && <circle cx={r1(spark.last[0])} cy={r1(spark.last[1])} r={2.75} fill={theme.palette[0]} />}
      </>
    ),
  }
}

function draw(ctx: Ctx): Drawn {
  if (ctx.width < 60) return emptyDrawn(Math.max(60, ctx.width), ctx.theme, '')
  switch (ctx.type) {
    case 'pie':
    case 'donut':
      return drawPie(ctx)
    case 'funnel':
      return drawFunnel(ctx)
    case 'heatmap':
      return drawHeatmap(ctx)
    case 'box':
      return drawBox(ctx)
    case 'radar':
      return drawRadar(ctx)
    case 'scatter':
    case 'geo':
      return drawScatter(ctx)
    case 'kpi':
      return drawKpi(ctx)
    case 'sparkline':
      return drawSpark(ctx)
    case 'histogram':
      return drawHistogram(ctx)
    default: {
      const vis = visible(ctx)
      if (vis.length === 0) return emptyDrawn(ctx.width, ctx.theme, 'Tüm seriler gizli; lejanttan açabilirsiniz')
      return drawCartesian(ctx, ctx.spec.labels, vis, ctx.type)
    }
  }
}

// ---------- Etkileşimli görünüm ----------

/** Birim eki: yüzde Türkçede başa yazılır (%12), diğerleri sona (12 TL). */
export function valueFormatter(unit: string | undefined): (v: number | null | undefined) => string {
  return (v) => {
    const s = formatNumber(v)
    if (!unit || s === '–') return s
    return unit === '%' ? percentText(s) : `${s} ${unit}`
  }
}

interface ChartViewProps {
  spec: ChartSpec
  type: ChartType
  hidden: ReadonlySet<string>
  zoom: ZoomWindow
  onZoom: (w: ZoomWindow) => void
  width: number
  theme: ChartTheme
  /** Ekran okuyucu için grafik özeti (svg aria-label) */
  label: string
  /** g17 dışa aktarma menüsünün okuduğu, yalnızca grafik SVG'sini içeren kap */
  hostRef?: React.RefObject<HTMLDivElement | null>
}

/** Grafik SVG'si + ipucu + klavye/işaretçi etkileşimi (ok tuşları, +/- yakınlaştırma, Ctrl+tekerlek, sürükleyerek kaydırma). */
export function ChartView({ spec, type, hidden, zoom, onZoom, width, theme, label, hostRef }: ChartViewProps) {
  const [active, setActive] = useState<number | null>(null)
  const [announce, setAnnounce] = useState('')
  const svgRef = useRef<SVGSVGElement>(null)
  const drag = useRef<{ x: number; zoom: ZoomWindow } | null>(null)
  const fmt = valueFormatter(spec.unit)
  const drawn = draw({ spec, type, width, theme, hidden, zoom, active, hover: setActive, fmt })
  const zoomable = ZOOMABLE.has(type)
  const live = useRef({ drawn, zoom, onZoom })
  live.current = { drawn, zoom, onZoom }

  const toSvg = (clientX: number, clientY: number): [number, number] | null => {
    const svg = svgRef.current
    if (!svg) return null
    const r = svg.getBoundingClientRect()
    const k = live.current.drawn.width / (r.width || 1)
    return [(clientX - r.left) * k, (clientY - r.top) * k]
  }

  // Tekerlek yalnızca Ctrl/Cmd ile yakınlaştırır (sohbet kaydırması ele geçirilmez); pasif olmayan dinleyici gerekir.
  useEffect(() => {
    const svg = svgRef.current
    if (!svg || !zoomable) return
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return
      e.preventDefault()
      const { drawn: d, zoom: z, onZoom: set } = live.current
      const p = toSvg(e.clientX, e.clientY)
      const anchor = p ? Math.min(1, Math.max(0, (p[0] - d.plot.x0) / Math.max(1, d.plot.x1 - d.plot.x0))) : 0.5
      set(zoomWindow(z, e.deltaY < 0 ? 0.8 : 1.25, anchor))
    }
    svg.addEventListener('wheel', onWheel, { passive: false })
    return () => svg.removeEventListener('wheel', onWheel)
  }, [zoomable])

  const locate = (clientX: number, clientY: number): number | null => {
    const p = toSvg(clientX, clientY)
    const { targets, mode, center } = drawn
    if (!p || targets.length === 0) return null
    const [x, y] = p
    if (mode === 'x') {
      let best = 0
      targets.forEach((t, i) => {
        if (Math.abs(t.x - x) < Math.abs(targets[best]!.x - x)) best = i
      })
      return best
    }
    if (mode === 'xy') {
      let best: number | null = null
      let bd = 24 * 24
      targets.forEach((t, i) => {
        const d = (t.x - x) ** 2 + (t.y - y) ** 2
        if (d <= bd) {
          bd = d
          best = i
        }
      })
      return best
    }
    if (mode === 'angle' && center) {
      const a = Math.atan2(y - center.cy, x - center.cx) + Math.PI / 2
      return (((Math.round(a / ((2 * Math.PI) / center.n)) % center.n) + center.n) % center.n) as number
    }
    return active
  }

  const onPointerMove = (e: PointerEvent<SVGSVGElement>) => {
    const d = drag.current
    if (d) {
      const p = toSvg(e.clientX, e.clientY)
      if (!p) return
      const frac = ((p[0] - d.x) / Math.max(1, drawn.plot.x1 - drawn.plot.x0)) * (d.zoom.end - d.zoom.start)
      onZoom(panWindow(d.zoom, -frac))
      return
    }
    if (drawn.mode !== 'mark') setActive(locate(e.clientX, e.clientY))
  }
  const zoomed = zoom.start > 0 || zoom.end < 1
  const onPointerDown = (e: PointerEvent<SVGSVGElement>) => {
    if (!zoomable || !zoomed || e.button !== 0) return
    const p = toSvg(e.clientX, e.clientY)
    if (!p) return
    drag.current = { x: p[0], zoom }
    e.currentTarget.setPointerCapture(e.pointerId)
  }
  const endDrag = () => {
    drag.current = null
  }

  const describe = (t: Target) => `${t.title}: ${t.rows.map((r) => `${r.name} ${r.value}`).join(', ')}`
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const n = drawn.targets.length
    const move = (next: number) => {
      const i = ((next % n) + n) % n
      setActive(i)
      const t = drawn.targets[i]
      if (t) setAnnounce(describe(t))
    }
    if ((e.key === 'ArrowRight' || e.key === 'ArrowDown') && n) move((active ?? -1) + 1)
    else if ((e.key === 'ArrowLeft' || e.key === 'ArrowUp') && n) move((active ?? 0) - 1)
    else if (e.key === 'Home' && n) move(0)
    else if (e.key === 'End' && n) move(n - 1)
    else if ((e.key === '+' || e.key === '=') && zoomable) onZoom(zoomWindow(zoom, 0.8))
    else if (e.key === '-' && zoomable) onZoom(zoomWindow(zoom, 1.25))
    else if (e.key === '0' && zoomable) onZoom(FULL_WINDOW)
    else if (e.key === 'Escape' && active !== null) setActive(null)
    else return
    e.preventDefault()
  }

  const tip = active !== null ? drawn.targets[active] : undefined
  const tipClass = tip ? `g18-tip${tip.x > drawn.width * 0.72 ? ' g18-tip-end' : tip.x < drawn.width * 0.28 ? ' g18-tip-start' : ''}${tip.y < 70 ? ' g18-tip-below' : ''}` : ''
  const interactive = drawn.targets.length > 0 || zoomable

  return (
    <div
      className={`g18-plot${zoomed ? ' g18-zoomed' : ''}`}
      tabIndex={interactive ? 0 : undefined}
      role={interactive ? 'group' : undefined}
      aria-label={interactive ? `${label}. Ok tuşlarıyla veri noktaları arasında gezinin${zoomable ? '; + ve - ile yakınlaştırın, 0 ile sıfırlayın' : ''}.` : undefined}
      onKeyDown={interactive ? onKeyDown : undefined}
      onBlur={() => setActive(null)}
    >
      <div ref={hostRef} className="g18-svg-host">
        <svg
          ref={svgRef}
          xmlns="http://www.w3.org/2000/svg"
          width={drawn.width}
          height={drawn.height}
          viewBox={`0 0 ${drawn.width} ${drawn.height}`}
          fontFamily={FONT}
          role="img"
          aria-label={label}
          onPointerMove={onPointerMove}
          onPointerDown={onPointerDown}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          onPointerLeave={() => {
            if (!drag.current) setActive(null)
          }}
        >
          <title>{label}</title>
          {drawn.marks}
        </svg>
      </div>
      {tip && (
        <div className={tipClass} style={{ left: `${(tip.x / drawn.width) * 100}%`, top: `${(tip.y / drawn.height) * 100}%` }} aria-hidden="true">
          <strong>{tip.title}</strong>
          {tip.rows.map((r, i) => (
            <span key={i} className="g18-tip-row">
              {r.color && <span className="g18-swatch" style={{ background: r.color }} />}
              <span className="g18-tip-name">{r.name}</span>
              <span className="g18-tip-value">{r.value}</span>
            </span>
          ))}
        </div>
      )}
      <span className="sr-only" aria-live="polite">
        {announce}
      </span>
    </div>
  )
}
