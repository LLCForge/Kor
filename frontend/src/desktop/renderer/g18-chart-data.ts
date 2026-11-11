// [g18] ```chart bloğu girdisini (JSON şartname, Chart.js benzeri JSON, nesne dizisi, CSV/TSV/;/boru tablosu ya da
//       düz sayı listesi) tek bir normalize grafik modeline çevirir. Saf ve DOM'suz; birim test edilir.
//       Güvenlik: model çıktısı güvenilmezdir; JSON zod ile doğrulanır, eval yok, boyut/seri/nokta sınırları var.
import { z } from 'zod'
import { finite } from './g18-chart-math.ts'
import { parseTable, type TableData } from './rich-data.ts'
import { cssColor, withinRichSizeLimit } from './rich-sanitize.ts'

export const CHART_TYPES = [
  'line',
  'bar',
  'stacked',
  'area',
  'scatter',
  'pie',
  'donut',
  'heatmap',
  'box',
  'radar',
  'waterfall',
  'funnel',
  'histogram',
  'dual',
  'kpi',
  'sparkline',
  'geo',
] as const
export type ChartType = (typeof CHART_TYPES)[number]

export const CHART_TYPE_LABELS: Record<ChartType, string> = {
  line: 'Çizgi',
  bar: 'Sütun',
  stacked: 'Yığılmış sütun',
  area: 'Alan',
  scatter: 'Dağılım',
  pie: 'Pasta',
  donut: 'Halka',
  heatmap: 'Isı haritası',
  box: 'Kutu',
  radar: 'Radar',
  waterfall: 'Şelale',
  funnel: 'Huni',
  histogram: 'Histogram',
  dual: 'Çift eksen',
  kpi: 'KPI kartı',
  sparkline: 'Mini grafik',
  geo: 'Coğrafi nokta',
}

/** DoS sınırları: seri (ısı haritasında satır), seri başına nokta ve toplam değer sayısı. */
export const MAX_SERIES = 50
export const MAX_POINTS = 5000
export const MAX_VALUES = 10_000

export interface ChartSeries {
  name: string
  values: (number | null)[]
  /** Sayısal x (dağılım) ya da boylam (coğrafi); values ile aynı uzunlukta. */
  xs?: (number | null)[]
  pointLabels?: string[]
  axis: 'left' | 'right'
  kind?: 'line' | 'bar'
  color?: string
}

export type RefStat = 'mean' | 'median' | 'min' | 'max'

export interface RefLine {
  value: number | RefStat
  label?: string
  axis: 'left' | 'right'
}

export interface KpiSpec {
  value?: number
  previous?: number
  label?: string
  lowerIsBetter: boolean
}

export interface ChartSpec {
  type: ChartType
  title?: string
  labels: string[]
  series: ChartSeries[]
  stacked: boolean
  xLabel?: string
  yLabel?: string
  y2Label?: string
  refLines: RefLine[]
  bins?: number
  unit?: string
  kpi?: KpiSpec
}

// ---------- Sayı ve tür normalizasyonu ----------

/**
 * Hücre/JSON değerini sayıya çevirir: para birimi/yüzde/birim ekleri atılır; Türkçe (1.234,5) ve İngilizce
 * (1,234.5) ayraçları çözülür. Tek virgül ondalık sayılır, yalnızca 1,234 / 12,345,678 kalıbı binlik ayraçtır.
 */
export function toNumber(raw: unknown): number | null {
  if (typeof raw === 'number') return Number.isFinite(raw) ? raw : null
  if (typeof raw !== 'string') return null
  let s = raw
    .replace(/[\s  ]/g, '')
    .replace(/^[^\d+\-.,]+/, '')
    .replace(/[^\d.,]+$/, '')
  if (s === '') return null
  const dot = s.lastIndexOf('.')
  const comma = s.lastIndexOf(',')
  if (dot >= 0 && comma >= 0) s = comma > dot ? s.replace(/\./g, '').replace(',', '.') : s.replace(/,/g, '')
  else if (comma >= 0) s = /^[+-]?\d{1,3}(,\d{3})+$/.test(s) ? s.replace(/,/g, '') : s.replace(',', '.')
  else if (/^[+-]?\d{1,3}(\.\d{3}){2,}$/.test(s)) s = s.replace(/\./g, '')
  if (!/^[+-]?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?$/i.test(s)) return null
  const n = Number(s)
  return Number.isFinite(n) ? n : null
}

const TYPE_ALIASES: Record<string, ChartType> = {
  line: 'line', lines: 'line', cizgi: 'line', çizgi: 'line', timeseries: 'line',
  bar: 'bar', column: 'bar', columns: 'bar', sutun: 'bar', sütun: 'bar', groupedbar: 'bar',
  stacked: 'stacked', stackedbar: 'stacked', stackedcolumn: 'stacked', yigilmis: 'stacked', yığılmış: 'stacked',
  area: 'area', alan: 'area',
  scatter: 'scatter', point: 'scatter', points: 'scatter', bubble: 'scatter', dagilim: 'scatter', dağılım: 'scatter',
  pie: 'pie', pasta: 'pie', polararea: 'pie',
  donut: 'donut', doughnut: 'donut', ring: 'donut', halka: 'donut',
  heatmap: 'heatmap', heat: 'heatmap', matrix: 'heatmap', isi: 'heatmap', ısı: 'heatmap', isiharitasi: 'heatmap', ısıharitası: 'heatmap',
  box: 'box', boxplot: 'box', kutu: 'box',
  radar: 'radar', spider: 'radar', orumcek: 'radar', örümcek: 'radar',
  waterfall: 'waterfall', cascade: 'waterfall', bridge: 'waterfall', selale: 'waterfall', şelale: 'waterfall',
  funnel: 'funnel', huni: 'funnel',
  histogram: 'histogram', hist: 'histogram',
  dual: 'dual', combo: 'dual', mixed: 'dual', dualaxis: 'dual', multiaxis: 'dual', cifteksen: 'dual', çifteksen: 'dual',
  kpi: 'kpi', stat: 'kpi', metric: 'kpi', number: 'kpi', sayac: 'kpi', sayaç: 'kpi',
  sparkline: 'sparkline', spark: 'sparkline',
  geo: 'geo', map: 'geo', harita: 'geo', konum: 'geo',
}

/** Tür adını (İngilizce/Türkçe/Chart.js eş anlamlıları) bilinen türe çevirir; tanınmazsa undefined (otomatik seçilir). */
export function normalizeType(raw: unknown): ChartType | undefined {
  if (typeof raw !== 'string') return undefined
  return TYPE_ALIASES[raw.trim().toLocaleLowerCase('tr').replace(/[\s_-]+/g, '')]
}

function refValue(raw: unknown): number | RefStat | null {
  if (typeof raw === 'string') {
    const key = raw.trim().toLocaleLowerCase('tr')
    if (/^(mean|avg|average|ortalama)$/.test(key)) return 'mean'
    if (/^(median|medyan)$/.test(key)) return 'median'
    if (/^(min|minimum|en düşük)$/.test(key)) return 'min'
    if (/^(max|maks|maximum|en yüksek)$/.test(key)) return 'max'
  }
  return toNumber(raw)
}

const isRight = (axis: string | undefined) => !!axis && /^(right|r|y1|y2|sağ|sag|secondary|ikincil)$/i.test(axis.trim())

// ---------- JSON şeması (zod) ----------

const Text = z.string().max(300)
const Label = z.union([z.string().max(300), z.number()]).transform(String)
const Labels = z.array(Label).max(MAX_POINTS)
const Points = z.array(z.unknown()).max(MAX_POINTS)
const Num = z.union([z.number(), z.string().max(64), z.null()])
const AxisIn = z.union([Text, z.object({ label: Text.optional(), title: Text.optional() })])
const SeriesIn = z.object({
  name: Text.optional(),
  label: Text.optional(),
  data: Points.optional(),
  values: Points.optional(),
  axis: z.string().max(12).optional(),
  yAxisID: z.string().max(40).optional(),
  type: z.string().max(30).optional(),
  color: z.string().max(64).optional(),
})
const RefIn = z.object({ y: Num.optional(), value: Num.optional(), label: Text.optional(), axis: z.string().max(12).optional() })
const RefList = z.array(RefIn).max(20)
const SeriesList = z.array(SeriesIn).max(MAX_SERIES)

const SpecIn = z.object({
  type: z.string().max(30).optional(),
  title: z.union([Text, z.object({ text: Text })]).optional(),
  labels: Labels.optional(),
  categories: Labels.optional(),
  series: SeriesList.optional(),
  datasets: SeriesList.optional(),
  data: z.union([Points, z.object({ labels: Labels.optional(), datasets: SeriesList.optional() })]).optional(),
  values: Points.optional(),
  stacked: z.boolean().optional(),
  x: AxisIn.optional(),
  y: AxisIn.optional(),
  y2: AxisIn.optional(),
  xLabel: Text.optional(),
  yLabel: Text.optional(),
  y2Label: Text.optional(),
  refLines: RefList.optional(),
  referenceLines: RefList.optional(),
  thresholds: RefList.optional(),
  bins: z.number().int().min(1).max(50).optional(),
  unit: z.string().max(24).optional(),
  value: Num.optional(),
  previous: Num.optional(),
  label: Text.optional(),
  lowerIsBetter: z.boolean().optional(),
  matrix: z.array(Points).max(MAX_SERIES).optional(),
  rows: Labels.optional(),
  cols: Labels.optional(),
  points: Points.optional(),
  options: z.object({ plugins: z.object({ title: z.object({ text: z.unknown() }).optional() }).optional() }).optional(),
})
type SpecInput = z.infer<typeof SpecIn>

const axisText = (a: SpecInput['x']) => (typeof a === 'string' ? a : (a?.label ?? a?.title))

// ---------- Nokta dönüştürme ----------

interface ConvertedPoints {
  values: (number | null)[]
  xs: (number | null)[] | undefined
  xLabels: string[] | undefined
  pointLabels: string[] | undefined
  geo: boolean
}

const pick = (o: Record<string, unknown>, ...keys: string[]) => keys.map((k) => o[k]).find((v) => v !== undefined && v !== null)

/** Nokta dizisini y değerlerine, isteğe bağlı sayısal x'lere ve kategori etiketlerine ayırır. */
function convertPoints(data: readonly unknown[]): ConvertedPoints {
  const values: (number | null)[] = []
  const xs: (number | null)[] = []
  const xLabels: string[] = []
  const pointLabels: string[] = []
  let numericX = 0
  let textX = 0
  let geo = 0
  for (const p of data) {
    let x: number | null = null
    let xLabel = ''
    let label = ''
    let y: number | null = null
    if (Array.isArray(p)) {
      const nx = toNumber(p[0])
      if (p.length >= 2) {
        if (nx !== null) x = nx
        else xLabel = String(p[0] ?? '')
        y = toNumber(p[1])
        if (typeof p[2] === 'string') label = p[2].slice(0, 300)
      } else y = nx
    } else if (p !== null && typeof p === 'object') {
      const o = p as Record<string, unknown>
      const lat = toNumber(pick(o, 'lat', 'latitude', 'enlem'))
      const lon = toNumber(pick(o, 'lon', 'lng', 'long', 'longitude', 'boylam'))
      const name = pick(o, 'label', 'name', 'ad', 'city')
      if (lat !== null && lon !== null) {
        geo++
        x = lon
        y = lat
        label = typeof name === 'string' ? name.slice(0, 300) : ''
      } else {
        const rawX = o.x ?? name ?? o.category
        y = toNumber(pick(o, 'y', 'value', 'v', 'değer', 'deger'))
        if (typeof rawX === 'number' && Number.isFinite(rawX)) x = rawX
        else if (rawX !== undefined && rawX !== null) xLabel = String(rawX).slice(0, 300)
        if (o.x !== undefined && typeof name === 'string') label = name.slice(0, 300)
      }
    } else y = toNumber(p)
    if (x !== null) numericX++
    if (xLabel) textX++
    values.push(y)
    xs.push(x)
    xLabels.push(xLabel)
    pointLabels.push(label)
  }
  return {
    values,
    xs: numericX > 0 && numericX === data.length ? xs : undefined,
    xLabels: textX > 0 ? xLabels : undefined,
    pointLabels: pointLabels.some(Boolean) ? pointLabels : undefined,
    geo: geo > 0 && geo === data.length,
  }
}

// ---------- Tablo → model ----------

const LAT = /^(lat|latitude|enlem)$/i
const LON = /^(lon|lng|long|longitude|boylam)$/i
const DATE_LIKE = /^(\d{4}[-/.]\d{1,2}|\d{1,2}[-/.]\d{1,2}[-/.]\d{2,4}|q[1-4]|\d{4}$|(oca|şub|sub|mar|nis|may|haz|tem|ağu|agu|eyl|eki|kas|ara|jan|feb|apr|jun|jul|aug|sep|oct|nov|dec))/i

interface TableOptions {
  type?: ChartType
  title?: string
}

/** Tablo verisinden grafik modeli: ilk sayısal olmayan sütun etiket, sayısal sütunlar seri olur. */
export function tableToSpec({ headers, rows }: TableData, opts: TableOptions = {}): ChartSpec {
  if (rows.length === 0 || headers.length === 0) throw new Error('Grafik için veri satırı yok')
  const columns = headers.map((name, c) => {
    const raw = rows.map((r) => r[c] ?? '')
    const values = raw.map(toNumber)
    const filled = raw.filter((v) => v.trim() !== '').length
    return { name: name.trim() || `Sütun ${c + 1}`, raw, values, numeric: filled > 0 && finite(values).length / filled >= 0.8 }
  })
  const base = { title: opts.title, stacked: false, refLines: [] as RefLine[] }
  const lat = columns.findIndex((c) => LAT.test(c.name))
  const lon = columns.findIndex((c) => LON.test(c.name))
  const labelCol = columns.findIndex((c) => !c.numeric)
  if (lat >= 0 && lon >= 0 && (!opts.type || opts.type === 'geo' || opts.type === 'scatter')) {
    const names = labelCol >= 0 ? columns[labelCol]!.raw : undefined
    return finalize({
      ...base,
      type: opts.type ?? 'geo',
      labels: [],
      series: [{ name: 'Konumlar', values: columns[lat]!.values, xs: columns[lon]!.values, pointLabels: names, axis: 'left' }],
      xLabel: 'Boylam',
      yLabel: 'Enlem',
    })
  }
  const numeric = columns.filter((c, i) => c.numeric && i !== labelCol)
  if (numeric.length === 0) throw new Error('Tabloda sayısal sütun bulunamadı')
  if (labelCol < 0 && numeric.length >= 2) {
    const [x, ...rest] = numeric as [(typeof numeric)[number], ...typeof numeric]
    const xs = x.values
    const sorted = xs.every((v, i) => i === 0 || (v !== null && xs[i - 1] !== null && v >= xs[i - 1]!))
    if (opts.type === 'scatter' || (!opts.type && !sorted)) {
      return finalize({ ...base, type: 'scatter', labels: [], series: rest.map((c) => ({ name: c.name, values: c.values, xs, axis: 'left' as const })), xLabel: x.name })
    }
    return finalize({ ...base, type: opts.type, labels: x.raw, series: rest.map((c) => ({ name: c.name, values: c.values, axis: 'left' as const })), xLabel: x.name })
  }
  const labels = labelCol >= 0 ? columns[labelCol]!.raw : rows.map((_, i) => String(i + 1))
  return finalize({
    ...base,
    type: opts.type,
    labels,
    series: numeric.map((c) => ({ name: c.name, values: c.values, axis: 'left' as const })),
    xLabel: labelCol >= 0 ? columns[labelCol]!.name : undefined,
  })
}

// shortcut: sekme/; ayrımlı metinde tırnak içi ayraç ve satır sonu desteklenmez (elektronik tablo yapıştırmaları için
//           yeterli); tırnaklı karmaşık CSV virgül ayraçlıysa rich-data parseTable'ın tam ayrıştırıcısına gider.
function splitDelimited(text: string, delim: string): string[][] {
  return text
    .split(/\r?\n/)
    .filter((l) => l.trim() !== '')
    .map((l) => l.split(delim).map((c) => c.trim().replace(/^"([\s\S]*)"$/, '$1').replace(/""/g, '"')))
}

function delimiterOf(line: string): '\t' | ';' | ',' | '|' | null {
  if (line.includes('\t')) return '\t'
  if (line.includes('|')) return '|'
  const semi = line.split(';').length
  const comma = line.split(',').length
  if (semi > 1 && semi >= comma) return ';'
  return comma > 1 ? ',' : null
}

/** CSV/TSV/;/boru metnini tabloya çevirir; ilk satır tamamen sayısalsa başlık uydurulur. */
export function parseDelimited(text: string): TableData {
  const trimmed = text.trim()
  const delim = delimiterOf(trimmed.split(/\r?\n/, 1)[0] ?? '')
  let matrix: string[][]
  if (delim === '\t' || delim === ';') matrix = splitDelimited(trimmed, delim)
  else {
    const t = parseTable(trimmed)
    matrix = [t.headers, ...t.rows]
  }
  const first = matrix[0] ?? []
  if (first.length > 0 && first.every((c) => toNumber(c) !== null)) {
    return { headers: first.map((_, i) => (i === 0 ? 'X' : `Seri ${i}`)), rows: matrix }
  }
  return { headers: first, rows: matrix.slice(1) }
}

// ---------- Ana ayrıştırıcı ----------

const DIRECTIVE = /^\s*(type|tür|tur|title|başlık|baslik|stacked|yığılmış|yigilmis|unit|birim|xlabel|ylabel|x|y)\s*[:=]\s*(.*)$/i

interface Directives {
  type?: ChartType
  title?: string
  stacked?: boolean
  unit?: string
  xLabel?: string
  yLabel?: string
}

/** CSV gövdesinin başındaki "type: bar", "başlık: Satışlar" gibi ayar satırlarını ayırır. */
function takeDirectives(code: string): { directives: Directives; body: string } {
  const lines = code.split(/\r?\n/)
  const directives: Directives = {}
  let i = 0
  for (; i < lines.length; i++) {
    const line = lines[i]!
    if (!line.trim()) continue
    const m = DIRECTIVE.exec(line)
    if (!m || line.trim().startsWith('{') || line.trim().startsWith('[')) break
    const key = m[1]!.toLocaleLowerCase('tr')
    const value = m[2]!.trim().slice(0, 300)
    if (key === 'type' || key === 'tür' || key === 'tur') directives.type = normalizeType(value)
    else if (key === 'title' || key === 'başlık' || key === 'baslik') directives.title = value
    else if (key === 'unit' || key === 'birim') directives.unit = value.slice(0, 24)
    else if (key === 'x' || key === 'xlabel') directives.xLabel = value
    else if (key === 'y' || key === 'ylabel') directives.yLabel = value
    else directives.stacked = /^(true|evet|1|yes)$/i.test(value)
  }
  return { directives, body: lines.slice(i).join('\n') }
}

/** "1 2 3", "1,2,3", "1,5; 2,5" ya da satır başına bir sayı biçimindeki düz sayı listesi; değilse null. */
export function parseNumberList(body: string): number[] | null {
  const lines = body
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean)
  if (lines.length === 0) return null
  let tokens: string[]
  if (lines.length === 1) {
    const line = lines[0]!
    tokens = /\s/.test(line) || line.includes(';') ? line.split(/[\s;]+/).map((t) => t.replace(/,$/, '')) : line.split(',')
  } else {
    if (lines.some((l) => /[\t;]/.test(l))) return null
    tokens = lines
  }
  tokens = tokens.filter(Boolean)
  const nums = tokens.map(toNumber)
  return nums.length > 0 && nums.every((n) => n !== null) ? (nums as number[]) : null
}

function seriesFromInput(raw: z.infer<typeof SeriesIn>, i: number): { series: ChartSeries; conv: ConvertedPoints } {
  const conv = convertPoints(raw.data ?? raw.values ?? [])
  const kind = normalizeType(raw.type)
  return {
    conv,
    series: {
      name: (raw.name ?? raw.label ?? '').trim() || `Seri ${i + 1}`,
      values: conv.values,
      xs: conv.xs,
      pointLabels: conv.pointLabels,
      axis: isRight(raw.axis) || isRight(raw.yAxisID) ? 'right' : 'left',
      kind: kind === 'bar' || kind === 'stacked' ? 'bar' : kind === 'line' || kind === 'area' ? 'line' : undefined,
      color: cssColor(raw.color),
    },
  }
}

function fromJson(input: SpecInput, directives: Directives): ChartSpec {
  const pluginTitle = input.options?.plugins?.title?.text
  const title =
    directives.title ??
    (typeof input.title === 'string' ? input.title : input.title?.text) ??
    (typeof pluginTitle === 'string' ? pluginTitle.slice(0, 300) : undefined)
  const type = directives.type ?? normalizeType(input.type)
  const refLines: RefLine[] = []
  for (const r of [...(input.refLines ?? []), ...(input.referenceLines ?? []), ...(input.thresholds ?? [])]) {
    const value = refValue(r.y ?? r.value)
    if (value !== null) refLines.push({ value, label: r.label, axis: isRight(r.axis) ? 'right' : 'left' })
  }
  const common = {
    title,
    stacked: input.stacked ?? directives.stacked ?? false,
    xLabel: input.xLabel ?? axisText(input.x) ?? directives.xLabel,
    yLabel: input.yLabel ?? axisText(input.y) ?? directives.yLabel,
    y2Label: input.y2Label ?? axisText(input.y2),
    refLines,
    bins: input.bins,
    unit: input.unit ?? directives.unit,
  }
  const objectData = input.data && !Array.isArray(input.data) ? input.data : undefined
  const arrayData = Array.isArray(input.data) ? input.data : undefined
  let labels = input.labels ?? input.categories ?? objectData?.labels ?? input.cols ?? []
  const rawSeries = input.series ?? input.datasets ?? objectData?.datasets

  // Isı haritası matrisi: her satır bir seri, sütunlar etiket.
  if (input.matrix) {
    const series = input.matrix.map((row, i) => ({ name: input.rows?.[i] ?? `Satır ${i + 1}`, values: row.map(toNumber), axis: 'left' as const }))
    return finalize({ ...common, type: type ?? 'heatmap', labels, series })
  }
  // Nesne/dizi satırları (ör. [{"ay":"Oca","satış":10}]) tablo yoluna gider.
  if (!rawSeries && arrayData && arrayData.some((r) => r !== null && typeof r === 'object' && !Array.isArray(r))) {
    const table = parseTable(JSON.stringify(arrayData))
    const spec = tableToSpec(table, { type, title })
    return finalize({ ...spec, ...common, labels: spec.labels, series: spec.series, xLabel: common.xLabel ?? spec.xLabel, yLabel: common.yLabel ?? spec.yLabel })
  }
  let series: ChartSeries[] = []
  let geo = false
  if (rawSeries) {
    for (const [i, raw] of rawSeries.entries()) {
      const out = seriesFromInput(raw, i)
      if (labels.length === 0 && out.conv.xLabels) labels = out.conv.xLabels
      geo ||= out.conv.geo
      series.push(out.series)
    }
  } else if (input.points ?? arrayData ?? input.values) {
    const conv = convertPoints(input.points ?? arrayData ?? input.values ?? [])
    if (labels.length === 0 && conv.xLabels) labels = conv.xLabels
    geo = conv.geo
    series = [{ name: input.label ?? (geo ? 'Konumlar' : 'Değer'), values: conv.values, xs: conv.xs, pointLabels: conv.pointLabels, axis: 'left' }]
  }
  const kpiValue = toNumber(input.value ?? null)
  const kpi: KpiSpec | undefined =
    kpiValue !== null || type === 'kpi'
      ? { value: kpiValue ?? undefined, previous: toNumber(input.previous ?? null) ?? undefined, label: input.label, lowerIsBetter: input.lowerIsBetter ?? false }
      : undefined
  if (series.length === 0 && !kpi) throw new Error('Grafik için seri ya da veri bulunamadı')
  return finalize({ ...common, type: type ?? (kpi && series.length === 0 ? 'kpi' : geo ? 'geo' : undefined), labels, series, kpi })
}

type Draft = Omit<ChartSpec, 'type'> & { type: ChartType | undefined }

/** Etiketleri hizalar, sınırları uygular ve tür verilmemişse veriye bakarak seçer. */
function finalize(draft: Draft): ChartSpec {
  const series = draft.series.filter((s) => s.values.length > 0)
  if (series.length > MAX_SERIES) throw new Error(`En fazla ${MAX_SERIES} seri çizilebilir`)
  const total = series.reduce((n, s) => n + s.values.length, 0)
  if (total > MAX_VALUES || series.some((s) => s.values.length > MAX_POINTS)) throw new Error('Grafik çizilemeyecek kadar büyük')
  const n = Math.max(0, ...series.map((s) => s.values.length))
  const firstXs = series.find((s) => s.xs)?.xs
  let labels = draft.labels.slice(0, n).map((l) => l.slice(0, 300))
  if (labels.length === 0 && firstXs) labels = firstXs.map((x) => (x === null ? '' : String(x)))
  while (labels.length < n) labels.push(String(labels.length + 1))
  const allXs = series.length > 0 && series.every((s) => s.xs)
  const sortedXs = !!firstXs && firstXs.every((v, i) => i === 0 || (v !== null && firstXs[i - 1] !== null && v >= firstXs[i - 1]!))
  const type: ChartType =
    draft.type ??
    (allXs
      ? sortedXs && series.length === 1
        ? 'line'
        : 'scatter'
      : series.some((s) => s.axis === 'right')
        ? 'dual'
        : draft.stacked
          ? 'stacked'
          : labels.length > 12 || labels.some((l) => DATE_LIKE.test(l.trim()))
            ? 'line'
            : 'bar')
  return { ...draft, type: type === 'bar' && draft.stacked ? 'stacked' : type, labels, series }
}

/**
 * ```chart blok metnini grafik modeline çevirir.
 * @param forcedType blok dili türü zorluyorsa (ör. ```sparkline)
 * @throws boyut aşımı, geçersiz JSON/şema ya da çizilecek veri yoksa (Türkçe mesaj)
 */
export function parseChart(code: string, forcedType?: ChartType): ChartSpec {
  if (!withinRichSizeLimit(code)) throw new Error('İçerik görüntülenemeyecek kadar büyük')
  const { directives, body } = takeDirectives(code)
  if (forcedType) directives.type = forcedType
  const trimmed = body.trim()
  if (!trimmed) throw new Error('Grafik verisi boş')
  if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
    let json: unknown
    try {
      json = JSON.parse(trimmed)
    } catch {
      throw new Error('Geçersiz JSON')
    }
    const input = SpecIn.safeParse(Array.isArray(json) ? { data: json } : json)
    if (!input.success) {
      const issue = input.error.issues[0]
      throw new Error(`Geçersiz grafik şartnamesi${issue?.path.length ? ` (${issue.path.join('.')})` : ''}`)
    }
    return fromJson(input.data, directives)
  }
  const list = parseNumberList(trimmed)
  if (list) {
    return finalize({
      type: directives.type,
      title: directives.title,
      labels: [],
      series: [{ name: directives.yLabel ?? 'Değer', values: list, axis: 'left' }],
      stacked: false,
      refLines: [],
      unit: directives.unit,
    })
  }
  const spec = tableToSpec(parseDelimited(trimmed), { type: directives.type, title: directives.title })
  return {
    ...spec,
    stacked: directives.stacked ?? spec.stacked,
    unit: directives.unit,
    xLabel: directives.xLabel ?? spec.xLabel,
    yLabel: directives.yLabel ?? spec.yLabel,
    type: directives.stacked && spec.type === 'bar' ? 'stacked' : spec.type,
  }
}

// ---------- Model → tablo, tür seçenekleri, yapıştırma algısı ----------

const rawCell = (v: number | null | undefined) => (v === null || v === undefined ? '' : String(v))

/** Grafiğin arkasındaki veriyi tabloya çevirir (grafik→tablo görünümü, CSV kopyalama, ekran okuyucu alternatifi). */
export function specToTable(spec: ChartSpec, type: ChartType = spec.type, fmt: (v: number | null | undefined) => string = rawCell): TableData {
  if (type === 'kpi' && spec.series.length === 0) {
    return { headers: ['Gösterge', 'Değer'], rows: [[spec.kpi?.label ?? 'Değer', fmt(spec.kpi?.value)], ['Önceki', fmt(spec.kpi?.previous)]] }
  }
  if ((type === 'scatter' || type === 'geo') && spec.series.some((s) => s.xs)) {
    const geo = type === 'geo'
    return {
      headers: ['Seri', geo ? 'Boylam' : (spec.xLabel ?? 'X'), geo ? 'Enlem' : (spec.yLabel ?? 'Y'), 'Etiket'],
      rows: spec.series.flatMap((s) => s.values.map((v, i) => [s.name, fmt(s.xs?.[i]), fmt(v), s.pointLabels?.[i] ?? ''])),
    }
  }
  return {
    headers: [spec.xLabel ?? 'Etiket', ...spec.series.map((s) => s.name)],
    rows: spec.labels.map((label, i) => [label, ...spec.series.map((s) => fmt(s.values[i]))]),
  }
}

/** Aynı veriyle anlamlı çizilebilen türler (tür değiştirme menüsü). */
export function availableTypes(spec: ChartSpec): ChartType[] {
  if (spec.series.length === 0) return ['kpi']
  if (spec.type === 'geo') return ['geo', 'scatter']
  const multi = spec.series.length >= 2
  if (spec.series.every((s) => s.xs) && spec.type === 'scatter') return ['scatter', 'line', 'histogram', 'box']
  return CHART_TYPES.filter((t) => {
    if (t === 'geo' || t === 'scatter') return false
    if (t === 'stacked' || t === 'dual') return multi
    if (t === 'radar') return spec.labels.length >= 3
    return true
  })
}

/** Composer'a yapıştırılan metin grafiğe dönüştürülebilir bir tablo mu (CSV/TSV/; en az başlık + 2 satır)? */
export function isChartablePaste(text: string): boolean {
  const t = text.trim()
  if (!t || !withinRichSizeLimit(t) || t.startsWith('{') || t.startsWith('[') || t.startsWith('```')) return false
  const lines = t.split(/\r?\n/).filter((l) => l.trim())
  if (lines.length < 3) return false
  const delim = delimiterOf(lines[0]!)
  if (!delim || delim === '|') return false
  const cols = lines[0]!.split(delim).length
  if (lines.filter((l) => l.split(delim).length === cols).length < lines.length * 0.8) return false
  try {
    return parseChart(t).series.some((s) => finite(s.values).length >= 2)
  } catch {
    return false
  }
}
