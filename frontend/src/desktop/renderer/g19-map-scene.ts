// [g19] Tembel harita paketinin çekirdeği: paketli TopoJSON ülke sınırlarını (world-atlas, ağ yok) d3-geo ile
//      projekte eder ve React'in çizeceği düz bir sahne (SVG path dizeleri + piksel konumları) üretir. React
//      içermez; ana paket bu modülü yalnızca window.__g19Map üzerinden kullanır (React'in ikinci kopyası olmaz).
//      Sınırlar elle çizilmez: tek kaynak paketli Natural Earth 1:110m verisidir.
import { geoArea, geoBounds, geoDistance, geoGraticule10, geoLength, geoMercator, geoNaturalEarth1, geoPath, type GeoProjection } from 'd3-geo'
import type { Geometry, GeometryCollection, LineString, Polygon } from 'geojson'
import { feature } from 'topojson-client'
import world from 'world-atlas/countries-110m.json' with { type: 'json' }
import type { LonLat, MapModel, MapScene, ScenePoint } from './g19-geo.ts'

const WIDTH = 960
const HEIGHT = 500
const EARTH_KM = 6371.0088
/** Bölgesel görünümde en küçük kapsam (derece): tek nokta sonsuz yakınlaşmasın. */
const MIN_SPAN = 2

/** Ülke anahtarı: aksan, büyük/küçük harf ve noktalama duyarsız ("Türkiye" = "turkiye"); sayısal kod "004" = "4". */
export function countryKey(s: string): string {
  const t = s.trim()
  if (/^\d+$/.test(t)) return String(Number(t))
  return t
    .normalize('NFKD')
    .replace(/\p{M}/gu, '')
    .toLowerCase()
    .replace(/ı/g, 'i')
    .replace(/[^\p{L}\p{N}]+/gu, '')
}

/** ISO 3166 sayısal kodu CLDR takma adlarıyla alfa-2'ye çevirir ("792" -> "TR"); gömülü kod tablosu gerekmez. */
function alpha2(numeric: string): string | undefined {
  try {
    const region = Intl.getCanonicalLocales(`und-${numeric}`)[0]?.split('-')[1]
    return region && /^[A-Z]{2}$/.test(region) ? region : undefined
  } catch {
    return undefined
  }
}

/**
 * Ülke arama dizini: sayısal kod, alfa-2, atlas adı, İngilizce (uzun/kısa) ve Türkçe CLDR adları aynı ülkeye
 * eşlenir. Görünen ad Türkçe CLDR adıdır (yoksa atlas adı). Çakışmada ilk eşleşme kazanır.
 */
export function buildCountryIndex(entries: readonly { id?: string; name: string }[]): { index: Map<string, number>; names: string[] } {
  const en = new Intl.DisplayNames(['en'], { type: 'region' })
  const enShort = new Intl.DisplayNames(['en'], { type: 'region', style: 'short' })
  const tr = new Intl.DisplayNames(['tr'], { type: 'region' })
  const index = new Map<string, number>()
  const names: string[] = []
  entries.forEach((entry, i) => {
    const a2 = entry.id ? alpha2(entry.id) : undefined
    const trName = a2 ? tr.of(a2) : undefined
    names.push(trName && trName !== a2 ? trName : entry.name)
    for (const k of [entry.id, entry.name, a2, a2 && en.of(a2), a2 && enShort.of(a2), trName]) {
      if (!k) continue
      const key = countryKey(k)
      if (key && !index.has(key)) index.set(key, i)
    }
  })
  return { index, names }
}

interface Countries {
  ids: string[]
  geometries: Geometry[]
  names: string[]
  index: Map<string, number>
}

let countries: Countries | null = null

function loadCountries(): Countries {
  if (countries) return countries
  const fc = feature(world, world.objects.countries)
  const entries = fc.features.map((f) => ({ id: f.id === undefined ? undefined : String(f.id), name: f.properties.name }))
  const { index, names } = buildCountryIndex(entries)
  countries = { ids: entries.map((e, i) => e.id ?? `x${i}`), geometries: fc.features.map((f) => f.geometry), names, index }
  return countries
}

/** Halkaları kapatır ve sarımı düzeltir: d3-geo dış halkayı saat yönünde bekler (RFC 7946'nın tersi). */
export function polygonGeometry(rings: readonly LonLat[][]): Polygon {
  const closed = rings.map((ring) => {
    const first = ring[0]
    const last = ring[ring.length - 1]
    return first && last && (first[0] !== last[0] || first[1] !== last[1]) ? [...ring, first] : ring
  })
  const polygon: Polygon = { type: 'Polygon', coordinates: closed }
  // Yarımküreden büyük alan = ters sarım (çokgen tüm dünyayı eksi kendisini kaplardı).
  return geoArea(polygon) > 2 * Math.PI ? { type: 'Polygon', coordinates: closed.map((r) => [...r].reverse()) } : polygon
}

function line(coords: LonLat[]): LineString {
  return { type: 'LineString', coordinates: coords }
}

function overlayOf(model: MapModel): GeometryCollection | null {
  const geometries: Geometry[] = []
  const points = [...model.markers, ...model.heat].map((p): LonLat => [p.lon, p.lat])
  if (points.length > 0) geometries.push({ type: 'MultiPoint', coordinates: points })
  for (const r of model.routes) geometries.push(line(r.coords))
  for (const p of model.polygons) geometries.push(polygonGeometry(p.rings))
  return geometries.length > 0 ? { type: 'GeometryCollection', geometries } : null
}

/** Veriye yakınlaşan Mercator; çok geniş ya da kutba yakın veride null (dünya görünümüne düşülür). */
function regionalProjection(overlay: GeometryCollection): GeoProjection | null {
  const [[west, south], [east, north]] = geoBounds(overlay)
  const lonSpan = east >= west ? east - west : east + 360 - west
  if (!Number.isFinite(lonSpan) || lonSpan > 200 || south < -80 || north > 80) return null
  const centerLon = west + lonSpan / 2
  const centerLat = (south + north) / 2
  const geometries: Geometry[] = [...overlay.geometries]
  // %20 pay: kenardaki işaretin sağa yazılan etiketi kırpılmasın.
  const padLon = lonSpan * 0.2
  const padLat = (north - south) * 0.2
  geometries.push({
    type: 'MultiPoint',
    coordinates: [
      [centerLon - lonSpan / 2 - padLon, Math.max(-85, south - padLat)],
      [centerLon + lonSpan / 2 + padLon, Math.min(85, north + padLat)],
    ],
  })
  if (lonSpan < MIN_SPAN || north - south < MIN_SPAN) {
    const h = MIN_SPAN / 2
    geometries.push({
      type: 'MultiPoint',
      coordinates: [
        [centerLon - h, Math.max(-85, centerLat - h)],
        [centerLon + h, Math.min(85, centerLat + h)],
      ],
    })
  }
  return (
    geoMercator()
      .rotate([-centerLon, 0])
      .fitExtent(
        [
          [32, 32],
          [WIDTH - 32, HEIGHT - 32],
        ],
        { type: 'GeometryCollection', geometries },
      )
      // Görünüm dışındaki ülkeler kırpılır: yakın görünümde SVG (ve dışa aktarılan dosya) şişmez.
      .clipExtent([
        [0, 0],
        [WIDTH, HEIGHT],
      ])
  )
}

const round = (v: number) => Math.round(v * 10) / 10

/** Modeli SVG sahnesine çevirir (viewBox 960x500). */
export function buildScene(model: MapModel): MapScene {
  const c = loadCountries()
  const values = new Map<number, number>()
  const unmatched: string[] = []
  for (const r of model.regions) {
    const i = c.index.get(countryKey(r.key))
    if (i === undefined) unmatched.push(r.key)
    else values.set(i, r.value)
  }

  const overlay = overlayOf(model)
  const regional = model.view === 'world' || model.regions.length > 0 || !overlay ? null : regionalProjection(overlay)
  const projection =
    regional ??
    geoNaturalEarth1().fitExtent(
      [
        [8, 8],
        [WIDTH - 8, HEIGHT - 8],
      ],
      { type: 'Sphere' },
    )
  const path = geoPath(projection).digits(1)

  const project = (p: LonLat): [number, number] | null => {
    const xy = projection(p)
    return xy && Number.isFinite(xy[0]) && Number.isFinite(xy[1]) ? [round(xy[0]), round(xy[1])] : null
  }
  const placed = (list: MapModel['markers']): ScenePoint[] =>
    list.flatMap((p) => {
      const xy = project([p.lon, p.lat])
      return xy ? [{ ...p, x: xy[0], y: xy[1] }] : []
    })

  let kmPerPx: number | null = null
  if (regional?.invert) {
    const a = regional.invert([WIDTH / 2 - 50, HEIGHT / 2])
    const b = regional.invert([WIDTH / 2 + 50, HEIGHT / 2])
    if (a && b) kmPerPx = (geoDistance(a, b) * EARTH_KM) / 100
  }

  return {
    width: WIDTH,
    height: HEIGHT,
    world: !regional,
    sphere: regional ? null : path({ type: 'Sphere' }),
    graticule: path(geoGraticule10()) ?? '',
    countries: c.geometries.map((g, i) => ({ id: c.ids[i] ?? `x${i}`, name: c.names[i] ?? '', d: path(g) ?? '', value: values.get(i) })),
    unmatched,
    markers: placed(model.markers),
    heat: placed(model.heat),
    routes: model.routes.map((r) => ({ ...r, d: path(line(r.coords)) ?? '', km: geoLength(line(r.coords)) * EARTH_KM })),
    polygons: model.polygons.map((p) => ({ ...p, d: path(polygonGeometry(p.rings)) ?? '' })),
    kmPerPx,
    project,
    invert(x, y) {
      const ll = projection.invert?.([x, y])
      if (!ll || !Number.isFinite(ll[0]) || !Number.isFinite(ll[1])) return null
      // Küre dışı tıklamalar (dünya görünümünde köşeler) geri projeksiyonda tutmaz.
      const back = projection(ll)
      if (!back || Math.hypot(back[0] - x, back[1] - y) > 1) return null
      const lon = ((((ll[0] + 180) % 360) + 360) % 360) - 180
      return Math.abs(ll[1]) <= 90 ? [lon, ll[1]] : null
    },
  }
}
