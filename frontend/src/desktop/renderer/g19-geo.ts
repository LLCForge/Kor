// [g19] Harita blokları (```map, ```geojson) için saf yardımcılar: koordinat ayrıştırma (ondalık/DMS), DMS
//      biçimlendirme, blok ayrıştırma (zod + elle GeoJSON yürüyüşü), renk ölçeği, ölçek çubuğu ve harita temaları.
//      DOM ve d3 yok; tembel harita paketi (g19-map-scene.ts) buradan yalnızca tür alır.
import { z } from 'zod'
import { uiLocale } from './ui-locale.ts'

/** GeoJSON sırası: [boylam, enlem]. */
export type LonLat = [number, number]

export interface MapPoint {
  lon: number
  lat: number
  label?: string
  value?: number
}

export interface MapLine {
  label?: string
  coords: LonLat[]
}

export interface MapPolygon {
  label?: string
  value?: number
  rings: LonLat[][]
}

export interface MapRegionValue {
  key: string
  value: number
}

export interface MapModel {
  title?: string
  unit?: string
  view: 'auto' | 'world'
  regions: MapRegionValue[]
  markers: MapPoint[]
  heat: MapPoint[]
  routes: MapLine[]
  polygons: MapPolygon[]
  /** Kaynak dış kiremit (tile) istedi; ağ yasak olduğundan yalnızca kullanıcıya not gösterilir. */
  tilesRequested: boolean
  /** Geçersiz olduğu için atlanan girdi sayısı. */
  skipped: number
}

export interface SceneCountry {
  id: string
  name: string
  d: string
  value?: number
}

export interface ScenePoint extends MapPoint {
  x: number
  y: number
}

export interface SceneLine extends MapLine {
  d: string
  km: number
}

export interface ScenePolygon extends MapPolygon {
  d: string
}

export interface MapScene {
  width: number
  height: number
  world: boolean
  sphere: string | null
  graticule: string
  countries: SceneCountry[]
  /** Hiçbir ülkeyle eşleşmeyen bölge anahtarları. */
  unmatched: string[]
  markers: ScenePoint[]
  heat: ScenePoint[]
  routes: SceneLine[]
  polygons: ScenePolygon[]
  /** Bölgesel görünümde harita merkezindeki km/piksel; dünya görünümünde null (ölçek çubuğu yanıltıcı olur). */
  kmPerPx: number | null
  project(p: LonLat): [number, number] | null
  invert(x: number, y: number): LonLat | null
}

/** Tembel paketin (dist/renderer/g19-map.js) window.__g19Map üzerinden sunduğu API. */
export interface G19MapApi {
  buildScene(model: MapModel): MapScene
}

/** Tek blokta izin verilen toplam koordinat sayısı (DoS: SVG düğüm patlaması engellenir). */
export const MAX_MAP_POINTS = 10_000

// ---------- Koordinat ayrıştırma ----------

export interface Coordinate {
  lat: number
  lon: number
}

const validLat = (v: number) => Number.isFinite(v) && v >= -90 && v <= 90
const validLon = (v: number) => Number.isFinite(v) && v >= -180 && v <= 180

// Bir bileşen: işaret, derece (ondalık olabilir), isteğe bağlı dakika ('), saniye ("), yarıküre harfi.
// Türkçe harfler de kabul edilir: K(uzey)=N, G(üney)=S, D(oğu)=E, B(atı)=W. Harf bir sözcüğün başı olamaz
// (ör. "28.9 Denizli" içindeki D yarıküre sayılmaz).
const COMPONENT =
  String.raw`(?<![\w.+-])([+-]?)(\d{1,3}(?:\.\d+)?)(?!\d)\s*[°º˚]?\s*` +
  String.raw`(?:(\d{1,2}(?:\.\d+)?)\s*['′’]\s*)?` +
  String.raw`(?:(\d{1,2}(?:\.\d+)?)\s*(?:["″”]|'')\s*)?` +
  String.raw`(?:([NSEWKGDB])(?![\p{L}\p{N}]))?`
const PAIR = new RegExp(`${COMPONENT}\\s*[,;/]?\\s*${COMPONENT}`, 'gu')

const LAT_LETTERS = new Set(['N', 'S', 'K', 'G'])
const NEGATIVE_LETTERS = new Set(['S', 'W', 'G', 'B'])

interface Component {
  value: number
  axis: 'lat' | 'lon' | null
}

function component(sign: string, deg: string, min: string | undefined, sec: string | undefined, hemi: string | undefined): Component | null {
  const m = min ? Number(min) : 0
  const s = sec ? Number(sec) : 0
  if (m >= 60 || s >= 60) return null
  // Dakika/saniye varsa derece tam sayı olmalı (41.5°30' anlamsız).
  if ((min || sec) && deg.includes('.')) return null
  let value = Number(deg) + m / 60 + s / 3600
  if (sign === '-' || (hemi && NEGATIVE_LETTERS.has(hemi))) value = -value
  return { value, axis: hemi ? (LAT_LETTERS.has(hemi) ? 'lat' : 'lon') : null }
}

/**
 * Metindeki ilk geçerli koordinat çiftini bulur. Ondalık ("41.0082, 28.9784") ve DMS (41°0'29.5"N 28°58'42.2"E)
 * biçimleri; yarıküre harfi yoksa sıra enlem, boylamdır. `rest` eşleşme dışındaki metindir (etiket olarak kullanılır).
 */
export function parseCoordinatePair(text: string): (Coordinate & { rest: string }) | null {
  for (const m of text.matchAll(PAIR)) {
    const a = component(m[1] ?? '', m[2] ?? '', m[3], m[4], m[5])
    const b = component(m[6] ?? '', m[7] ?? '', m[8], m[9], m[10])
    if (!a || !b) continue
    // İki bileşen aynı eksendeyse (ör. iki N) çift geçersiz.
    if (a.axis && a.axis === b.axis) continue
    const swap = a.axis === 'lon' || b.axis === 'lat'
    const lat = swap ? b.value : a.value
    const lon = swap ? a.value : b.value
    if (!validLat(lat) || !validLon(lon)) continue
    const index = m.index ?? 0
    const rest = `${text.slice(0, index)} ${text.slice(index + m[0].length)}`
    return { lat, lon, rest }
  }
  return null
}

/** Satır başı liste işaretini ve kenar ayraçlarını temizleyip etiketi döndürür. */
function cleanLabel(rest: string): string | undefined {
  const label = rest
    .replace(/^\s*(?:[-*•]|\d+[.)])\s+/u, '')
    .replace(/^[\s:|,;–—-]+|[\s:|,;–—-]+$/gu, '')
    .slice(0, 200)
  return label || undefined
}

// ---------- Biçimlendirme ----------

export type CoordFormat = 'decimal' | 'dms'

function dmsPart(value: number, positive: string, negative: string): string {
  const abs = Math.abs(value)
  let deg = Math.floor(abs)
  let min = Math.floor((abs - deg) * 60)
  let sec = Math.round(((abs - deg) * 60 - min) * 600) / 10
  // Yuvarlama taşması: 59.95" -> 60.0" olmasın.
  if (sec >= 60) {
    sec = 0
    min += 1
  }
  if (min >= 60) {
    min = 0
    deg += 1
  }
  return `${deg}°${String(min).padStart(2, '0')}'${sec.toFixed(1).padStart(4, '0')}"${value < 0 ? negative : positive}`
}

/** Enlem/boylamı DMS olarak biçimlendirir: 41°00'29.5"N 28°58'42.2"E (kopyalanabilir, harita uygulamalarıyla uyumlu). */
export function formatDms(lat: number, lon: number): string {
  return `${dmsPart(lat, 'N', 'S')} ${dmsPart(lon, 'E', 'W')}`
}

/** Ondalık derece: "41.00820, 28.97840" (enlem, boylam). */
export function formatDecimal(lat: number, lon: number, digits = 5): string {
  return `${lat.toFixed(digits)}, ${lon.toFixed(digits)}`
}

export function formatCoord(lat: number, lon: number, format: CoordFormat): string {
  return format === 'dms' ? formatDms(lat, lon) : formatDecimal(lat, lon)
}

export function formatValue(value: number, unit?: string): string {
  const text = value.toLocaleString(uiLocale(), { maximumFractionDigits: 2 })
  return unit ? `${text} ${unit}` : text
}

/** Mesafe metni: 1 km altı metre, üstü km. */
export function formatKm(km: number): string {
  if (km < 1) return `${Math.round(km * 1000).toLocaleString(uiLocale())} m`
  return `${(km < 10 ? Math.round(km * 10) / 10 : Math.round(km)).toLocaleString(uiLocale())} km`
}

const round6 = (v: number) => Math.round(v * 1e6) / 1e6

/** Elle çizilen çokgeni RFC 7946 GeoJSON Feature'a çevirir: dış halka saat yönünün tersine, halka kapalı. */
export function draftToGeoJson(draft: readonly LonLat[]): string {
  // Düzlemsel işaretli alan (shoelace): negatifse sarım saat yönündedir.
  const signed = draft.reduce((sum, [x1, y1], i) => {
    const [x2, y2] = draft[(i + 1) % draft.length] ?? [x1, y1]
    return sum + (x1 * y2 - x2 * y1)
  }, 0)
  const ring = (signed < 0 ? [...draft].reverse() : [...draft]).map(([lon, lat]) => [round6(lon), round6(lat)])
  const first = ring[0]
  if (first) ring.push(first)
  return JSON.stringify({ type: 'Feature', properties: {}, geometry: { type: 'Polygon', coordinates: [ring] } }, null, 2)
}

// ---------- Renk ölçeği ve ölçek çubuğu ----------

function hexToRgb(hex: string): [number, number, number] {
  const n = Number.parseInt(hex.slice(1), 16)
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255]
}

/** İki #rrggbb rengi arasında doğrusal karışım (t: 0..1). */
export function mixHex(from: string, to: string, t: number): string {
  const a = hexToRgb(from)
  const b = hexToRgb(to)
  const k = Math.min(1, Math.max(0, t))
  return `#${a.map((c, i) => Math.round(c + ((b[i] as number) - c) * k).toString(16).padStart(2, '0')).join('')}`
}

export interface ColorStep {
  from: number
  to: number
  color: string
}

export interface ColorScale {
  steps: ColorStep[]
  color(value: number): string
}

/**
 * Eşit aralıklı sınıflandırılmış (quantize) sıralı renk ölçeği. Tek tonlu açıklık rampası renk körlüğüne
 * dayanıklıdır. Değer yoksa null; tüm değerler eşitse tek sınıf.
 */
export function colorScale(values: readonly number[], ramp: readonly [string, string], classes = 5): ColorScale | null {
  const nums = values.filter(Number.isFinite)
  if (nums.length === 0) return null
  const min = Math.min(...nums)
  const max = Math.max(...nums)
  const n = min === max ? 1 : Math.max(1, Math.floor(classes))
  const width = (max - min) / n
  const steps: ColorStep[] = Array.from({ length: n }, (_, i) => ({
    from: min + width * i,
    to: i === n - 1 ? max : min + width * (i + 1),
    color: n === 1 ? ramp[1] : mixHex(ramp[0], ramp[1], i / (n - 1)),
  }))
  return {
    steps,
    color(value) {
      const i = width === 0 ? 0 : Math.min(n - 1, Math.max(0, Math.floor((value - min) / width)))
      return steps[i]?.color ?? ramp[1]
    },
  }
}

/** maxKm'yi aşmayan en büyük "yuvarlak" mesafe: 1, 2, 5 x 10^k. */
export function niceDistance(maxKm: number): number {
  if (!(maxKm > 0) || !Number.isFinite(maxKm)) return 0
  const pow = 10 ** Math.floor(Math.log10(maxKm))
  const base = maxKm / pow
  return (base >= 5 ? 5 : base >= 2 ? 2 : 1) * pow
}

/** Ölçek çubuğu: en fazla maxPx uzunluğunda yuvarlak mesafe ve piksel karşılığı. */
export function scaleBar(kmPerPx: number, maxPx = 120): { km: number; px: number } | null {
  const km = niceDistance(kmPerPx * maxPx)
  if (km <= 0) return null
  return { km, px: km / kmPerPx }
}

// ---------- Temalar ----------

export interface MapTheme {
  ocean: string
  land: string
  border: string
  grid: string
  text: string
  accent: string
  halo: string
  ramp: readonly [string, string]
}

/** Uygulama token'larıyla uyumlu iki harita teması (dışa aktarılan SVG'ye sunum öznitelikleri olarak girer). */
export const MAP_THEMES: Record<'light' | 'dark', MapTheme> = {
  light: { ocean: '#f4f1ea', land: '#ddd6c6', border: '#fbfaf6', grid: '#e4dfd2', text: '#3d3a35', accent: '#c96442', halo: '#ffffff', ramp: ['#f6dccf', '#8a3418'] },
  dark: { ocean: '#232220', land: '#3d3b36', border: '#232220', grid: '#2f2e2a', text: '#cdc9be', accent: '#d97757', halo: '#1f1e1b', ramp: ['#4a2c20', '#f4a582'] },
}

// ---------- Blok ayrıştırma ----------

export type ParseResult = { ok: true; model: MapModel } | { ok: false; error: string }

const finite = z.number()
const text = z.string().max(200)
const pointObject = z.object({
  lat: finite.optional(),
  latitude: finite.optional(),
  lon: finite.optional(),
  lng: finite.optional(),
  longitude: finite.optional(),
  label: text.optional(),
  name: text.optional(),
  title: text.optional(),
  value: finite.optional(),
  weight: finite.optional(),
})
const pointTuple = z.tuple([finite, finite], finite)
const regionKey = z.union([z.string().max(100), z.number()])
const regionsSchema = z.union([
  z.record(z.string().max(100), finite),
  z.array(z.object({ id: regionKey.optional(), code: regionKey.optional(), name: regionKey.optional(), country: regionKey.optional(), value: finite })),
])
const shapeObject = z.object({ label: text.optional(), name: text.optional(), value: finite.optional(), points: z.array(z.unknown()) })
const specSchema = z.object({
  title: text.optional(),
  unit: z.string().max(40).optional(),
  view: z.enum(['auto', 'world']).optional(),
  regions: regionsSchema.optional(),
  choropleth: regionsSchema.optional(),
  markers: z.array(z.unknown()).optional(),
  points: z.array(z.unknown()).optional(),
  heat: z.array(z.unknown()).optional(),
  route: z.array(z.unknown()).optional(),
  routes: z.array(z.unknown()).optional(),
  polygons: z.array(z.unknown()).optional(),
  geojson: z.unknown().optional(),
})

class Collector {
  readonly model: MapModel = { view: 'auto', regions: [], markers: [], heat: [], routes: [], polygons: [], tilesRequested: false, skipped: 0 }
  private count = 0

  /** Nokta bütçesini düşer; aşılırsa ayrıştırma durdurulur. */
  spend(n: number): void {
    this.count += n
    if (this.count > MAX_MAP_POINTS) throw new RangeError(`Harita en fazla ${MAX_MAP_POINTS.toLocaleString(uiLocale())} koordinat gösterebilir.`)
  }
}

/** ```map girdisindeki bir noktayı ([enlem, boylam, ağırlık?] | "41.0, 28.9" | {lat, lon, ...}) çözer. */
function toPoint(input: unknown): MapPoint | null {
  if (typeof input === 'string') {
    const c = parseCoordinatePair(input)
    return c ? { lat: c.lat, lon: c.lon, label: cleanLabel(c.rest) } : null
  }
  const tuple = pointTuple.safeParse(input)
  if (tuple.success) {
    const [lat, lon, value] = tuple.data
    return validLat(lat) && validLon(lon) ? { lat, lon, value } : null
  }
  const obj = pointObject.safeParse(input)
  if (!obj.success) return null
  const o = obj.data
  const lat = o.lat ?? o.latitude
  const lon = o.lon ?? o.lng ?? o.longitude
  if (lat === undefined || lon === undefined || !validLat(lat) || !validLon(lon)) return null
  return { lat, lon, label: o.label ?? o.name ?? o.title, value: o.value ?? o.weight }
}

function collectPoints(list: readonly unknown[], out: Collector): MapPoint[] {
  out.spend(list.length)
  const points: MapPoint[] = []
  for (const item of list) {
    const p = toPoint(item)
    if (p) points.push(p)
    else out.model.skipped++
  }
  return points
}

/** Bir çizgi/çokgen tanımını ([...noktalar] | {label, points}) etiket + koordinatlara çevirir. */
function collectShape(input: unknown, out: Collector): { label?: string; value?: number; coords: LonLat[] } | null {
  const shape = shapeObject.safeParse(input)
  const list = shape.success ? shape.data.points : Array.isArray(input) ? input : null
  if (!list) return null
  const coords = collectPoints(list, out).map((p): LonLat => [p.lon, p.lat])
  return { label: shape.success ? (shape.data.label ?? shape.data.name) : undefined, value: shape.success ? shape.data.value : undefined, coords }
}

function addRegions(regions: z.infer<typeof regionsSchema>, out: Collector): void {
  if (Array.isArray(regions)) {
    for (const r of regions) {
      const key = r.code ?? r.id ?? r.name ?? r.country
      if (key === undefined) out.model.skipped++
      else out.model.regions.push({ key: String(key), value: r.value })
    }
  } else {
    for (const [key, value] of Object.entries(regions)) out.model.regions.push({ key, value })
  }
}

// ---------- GeoJSON ----------

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v)

function lonLat(v: unknown): LonLat | null {
  if (!Array.isArray(v) || v.length < 2) return null
  const [lon, lat] = v
  return typeof lon === 'number' && typeof lat === 'number' && validLon(lon) && validLat(lat) ? [lon, lat] : null
}

function lonLatList(v: unknown, out: Collector): LonLat[] | null {
  if (!Array.isArray(v)) return null
  out.spend(v.length)
  const list: LonLat[] = []
  for (const item of v) {
    const p = lonLat(item)
    if (!p) return null
    list.push(p)
  }
  return list
}

interface FeatureInfo {
  label?: string
  value?: number
}

function featureInfo(props: unknown): FeatureInfo {
  if (!isRecord(props)) return {}
  const label = [props.name, props.title, props.label].find((v): v is string => typeof v === 'string')
  const value = typeof props.value === 'number' && Number.isFinite(props.value) ? props.value : undefined
  return { label: label?.slice(0, 200), value }
}

function addPolygon(rings: unknown, info: FeatureInfo, out: Collector): void {
  if (!Array.isArray(rings) || rings.length === 0) {
    out.model.skipped++
    return
  }
  const parsed: LonLat[][] = []
  for (const ring of rings) {
    const list = lonLatList(ring, out)
    if (!list || list.length < 3) {
      out.model.skipped++
      return
    }
    parsed.push(list)
  }
  out.model.polygons.push({ ...info, rings: parsed })
}

function addLine(coords: unknown, info: FeatureInfo, out: Collector): void {
  const list = lonLatList(coords, out)
  if (list && list.length >= 2) out.model.routes.push({ label: info.label, coords: list })
  else out.model.skipped++
}

/** GeoJSON düğümünü (FeatureCollection/Feature/geometri) yürür; derinlik iç içe GeometryCollection'a karşı sınırlı. */
function walkGeoJson(node: unknown, out: Collector, info: FeatureInfo = {}, depth = 0): void {
  if (!isRecord(node) || depth > 8) {
    out.model.skipped++
    return
  }
  const coords = node.coordinates
  switch (node.type) {
    case 'FeatureCollection':
      if (Array.isArray(node.features)) for (const f of node.features) walkGeoJson(f, out, {}, depth + 1)
      else out.model.skipped++
      return
    case 'Feature':
      if (node.geometry !== null) walkGeoJson(node.geometry, out, featureInfo(node.properties), depth + 1)
      return
    case 'GeometryCollection':
      if (Array.isArray(node.geometries)) for (const g of node.geometries) walkGeoJson(g, out, info, depth + 1)
      return
    case 'Point':
    case 'MultiPoint': {
      const list = node.type === 'Point' ? lonLatList([coords], out) : lonLatList(coords, out)
      if (!list) out.model.skipped++
      else for (const [lon, lat] of list) out.model.markers.push({ lon, lat, ...info })
      return
    }
    case 'LineString':
      addLine(coords, info, out)
      return
    case 'MultiLineString':
      if (Array.isArray(coords)) for (const line of coords) addLine(line, info, out)
      else out.model.skipped++
      return
    case 'Polygon':
      addPolygon(coords, info, out)
      return
    case 'MultiPolygon':
      if (Array.isArray(coords)) for (const poly of coords) addPolygon(poly, info, out)
      else out.model.skipped++
      return
    default:
      out.model.skipped++
  }
}

function run(fill: (out: Collector) => void): ParseResult {
  const out = new Collector()
  try {
    fill(out)
  } catch (err) {
    if (err instanceof RangeError) return { ok: false, error: err.message }
    throw err
  }
  const m = out.model
  if (m.regions.length + m.markers.length + m.heat.length + m.routes.length + m.polygons.length === 0) {
    return { ok: false, error: 'Haritada gösterilecek geçerli veri bulunamadı.' }
  }
  return { ok: true, model: m }
}

function parseJson(code: string): { ok: true; data: unknown } | { ok: false } {
  try {
    return { ok: true, data: JSON.parse(code) }
  } catch {
    return { ok: false }
  }
}

/** ```geojson bloğu: standart GeoJSON ([boylam, enlem] sırası). */
export function parseGeoJsonBlock(code: string): ParseResult {
  const json = parseJson(code)
  if (!json.ok) return { ok: false, error: 'Geçersiz GeoJSON (JSON ayrıştırılamadı).' }
  return run((out) => walkGeoJson(json.data, out))
}

/**
 * ```map bloğu. JSON nesnesi (regions/markers/heat/route(s)/polygons/geojson; diziler [enlem, boylam] sırasıdır),
 * nokta dizisi, ya da satır başına bir koordinat ("41.0082, 28.9784 İstanbul" veya DMS) içeren düz metin.
 */
export function parseMapBlock(code: string): ParseResult {
  const trimmed = code.trim()
  if (!/^[[{]/.test(trimmed)) {
    return run((out) => {
      const lines = trimmed.split(/\r?\n/).filter((l) => l.trim())
      out.spend(lines.length)
      for (const line of lines) {
        const c = parseCoordinatePair(line)
        if (c) out.model.markers.push({ lat: c.lat, lon: c.lon, label: cleanLabel(c.rest) })
        else out.model.skipped++
      }
    })
  }
  const json = parseJson(trimmed)
  if (!json.ok) return { ok: false, error: 'Geçersiz harita verisi (JSON ayrıştırılamadı).' }
  if (Array.isArray(json.data)) {
    const list = json.data
    return run((out) => out.model.markers.push(...collectPoints(list, out)))
  }
  const spec = specSchema.safeParse(json.data)
  if (!spec.success) return { ok: false, error: `Geçersiz harita verisi: ${spec.error.issues[0]?.path.map(String).join('.') || 'kök'} alanı hatalı.` }
  const s = spec.data
  const raw = isRecord(json.data) ? json.data : {}
  return run((out) => {
    const m = out.model
    m.title = s.title
    m.unit = s.unit
    m.view = s.view ?? 'auto'
    m.tilesRequested = Object.keys(raw).some((k) => /tile/i.test(k))
    for (const regions of [s.regions, s.choropleth]) if (regions) addRegions(regions, out)
    m.markers.push(...collectPoints([...(s.markers ?? []), ...(s.points ?? [])], out))
    m.heat.push(...collectPoints(s.heat ?? [], out))
    const routes = [...(s.route ? [s.route] : []), ...(s.routes ?? [])]
    for (const r of routes) {
      const shape = collectShape(r, out)
      if (shape && shape.coords.length >= 2) m.routes.push({ label: shape.label, coords: shape.coords })
      else m.skipped++
    }
    for (const p of s.polygons ?? []) {
      const shape = collectShape(p, out)
      if (shape && shape.coords.length >= 3) m.polygons.push({ label: shape.label, value: shape.value, rings: [shape.coords] })
      else m.skipped++
    }
    if (s.geojson !== undefined) walkGeoJson(s.geojson, out)
  })
}
