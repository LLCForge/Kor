// Proje grafı için küçük kuvvet yerleşimi (d3-force benzeri hız Verlet). Bağımlılık yok, DOM yok: düğüm testleri
// doğrudan çalıştırır. İtme bir ızgara üzerinden yalnızca komşu hücrelerde hesaplanır (O(n) ortalama), böylece
// birkaç bin düğüm kare başına milisaniyeler içinde kalır.

export interface Sim {
  n: number
  x: Float64Array
  y: Float64Array
  vx: Float64Array
  vy: Float64Array
  /** 1 = kullanıcı sürüklüyor, kuvvetler konuma dokunmaz */
  pinned: Uint8Array
  /** Düz kenar listesi: [s0, t0, s1, t1, ...] */
  links: Uint32Array
  /** Kenar başına yay gücü: 1 / min(derece) */
  linkStrength: Float64Array
  /** Topluluk dizini (kümelenme kuvveti) */
  group: Int32Array
  alpha: number
}

export const LINK_DISTANCE = 34
export const CHARGE = 60
export const CUTOFF = 90
const VELOCITY_DECAY = 0.4
const ALPHA_DECAY = 0.0228
export const ALPHA_MIN = 0.001
const GROUP_PULL = 0.03
const CENTER_PULL = 0.004

/** Altın açı sarmalı: deterministik, iç içe geçmeyen başlangıç konumları. */
function spiral(i: number, step: number): [number, number] {
  const r = step * Math.sqrt(i + 0.5)
  const a = i * 2.399963229728653
  return [r * Math.cos(a), r * Math.sin(a)]
}

/** Topluluk merkezleri büyük sarmalda, üyeler kendi merkezlerinin etrafında küçük sarmalda başlar. */
export function createSim(n: number, edges: ReadonlyArray<readonly [number, number]>, group: ArrayLike<number>): Sim {
  const sim: Sim = {
    n,
    x: new Float64Array(n),
    y: new Float64Array(n),
    vx: new Float64Array(n),
    vy: new Float64Array(n),
    pinned: new Uint8Array(n),
    links: new Uint32Array(0),
    linkStrength: new Float64Array(0),
    group: Int32Array.from({ length: n }, (_, i) => group[i] ?? 0),
    alpha: 1,
  }
  const groups = [...new Set(sim.group)].sort((a, b) => a - b)
  const order = new Map(groups.map((g, i) => [g, i]))
  const seen = new Map<number, number>()
  for (let i = 0; i < n; i++) {
    const g = sim.group[i]!
    const [cx, cy] = spiral(order.get(g) ?? 0, 160)
    const k = seen.get(g) ?? 0
    seen.set(g, k + 1)
    const [dx, dy] = spiral(k, 14)
    sim.x[i] = cx + dx
    sim.y[i] = cy + dy
  }
  setLinks(sim, edges)
  return sim
}

function setLinks(sim: Sim, edges: ReadonlyArray<readonly [number, number]>): void {
  const valid = edges.filter(([s, t]) => s !== t && s < sim.n && t < sim.n)
  const degree = new Uint32Array(sim.n)
  for (const [s, t] of valid) {
    degree[s]!++
    degree[t]!++
  }
  sim.links = new Uint32Array(valid.length * 2)
  sim.linkStrength = new Float64Array(valid.length)
  valid.forEach(([s, t], i) => {
    sim.links[i * 2] = s
    sim.links[i * 2 + 1] = t
    sim.linkStrength[i] = 1 / Math.max(1, Math.min(degree[s]!, degree[t]!))
  })
}

/** Aynı noktadaki düğümler için küçük, deterministik sapma (sıfıra bölme olmasın). */
function jiggle(i: number): number {
  return ((i * 2654435761) % 1000) / 1e6 + 1e-6
}

/** Bir adım; güncel alpha'yı döndürür. alpha < ALPHA_MIN olunca yerleşim durmuş sayılır. */
export function tick(sim: Sim): number {
  const { n, x, y, vx, vy, links, linkStrength, group, alpha } = sim
  // Yaylar
  for (let e = 0; e < linkStrength.length; e++) {
    const s = links[e * 2]!
    const t = links[e * 2 + 1]!
    let dx = x[t]! + vx[t]! - x[s]! - vx[s]!
    let dy = y[t]! + vy[t]! - y[s]! - vy[s]!
    if (dx === 0 && dy === 0) dx = jiggle(e)
    const l = Math.sqrt(dx * dx + dy * dy)
    const f = ((l - LINK_DISTANCE) / l) * alpha * linkStrength[e]!
    dx *= f
    dy *= f
    vx[t]! -= dx * 0.5
    vy[t]! -= dy * 0.5
    vx[s]! += dx * 0.5
    vy[s]! += dy * 0.5
  }
  // İtme: ızgara, yalnızca CUTOFF içindeki komşular
  const cells = new Map<number, number[]>()
  const key = (cx: number, cy: number) => cx * 73856093 + cy * 19349663
  for (let i = 0; i < n; i++) {
    const k = key(Math.floor(x[i]! / CUTOFF), Math.floor(y[i]! / CUTOFF))
    const list = cells.get(k)
    if (list) list.push(i)
    else cells.set(k, [i])
  }
  const cut2 = CUTOFF * CUTOFF
  for (let i = 0; i < n; i++) {
    const cx = Math.floor(x[i]! / CUTOFF)
    const cy = Math.floor(y[i]! / CUTOFF)
    for (let ox = -1; ox <= 1; ox++) {
      for (let oy = -1; oy <= 1; oy++) {
        const list = cells.get(key(cx + ox, cy + oy))
        if (!list) continue
        for (const j of list) {
          if (j <= i) continue
          let dx = x[j]! - x[i]!
          let dy = y[j]! - y[i]!
          let d2 = dx * dx + dy * dy
          if (d2 >= cut2) continue
          if (d2 === 0) {
            dx = jiggle(i + j)
            d2 = dx * dx
          }
          d2 = Math.max(d2, 1)
          const f = (CHARGE * alpha) / d2
          vx[i]! -= dx * f
          vy[i]! -= dy * f
          vx[j]! += dx * f
          vy[j]! += dy * f
        }
      }
    }
  }
  // Topluluk merkezine ve sahne merkezine hafif çekim
  const sum = new Map<number, [number, number, number]>()
  for (let i = 0; i < n; i++) {
    const s = sum.get(group[i]!) ?? [0, 0, 0]
    s[0] += x[i]!
    s[1] += y[i]!
    s[2]++
    sum.set(group[i]!, s)
  }
  for (let i = 0; i < n; i++) {
    const [gx, gy, c] = sum.get(group[i]!)!
    vx[i]! += (gx / c - x[i]!) * GROUP_PULL * alpha - x[i]! * CENTER_PULL * alpha
    vy[i]! += (gy / c - y[i]!) * GROUP_PULL * alpha - y[i]! * CENTER_PULL * alpha
  }
  for (let i = 0; i < n; i++) {
    if (sim.pinned[i]) {
      vx[i] = 0
      vy[i] = 0
      continue
    }
    vx[i]! *= 1 - VELOCITY_DECAY
    vy[i]! *= 1 - VELOCITY_DECAY
    x[i]! += vx[i]!
    y[i]! += vy[i]!
  }
  sim.alpha = alpha + (0 - alpha) * ALPHA_DECAY
  return sim.alpha
}

/** Önceki yerleşimdeki konumları kimliğe göre yeni yerleşime taşır; taşınan düğüm sayısını döndürür. */
export function adoptPositions(sim: Sim, ids: readonly string[], from: Sim, fromIds: readonly string[]): number {
  const old = new Map(fromIds.map((id, i) => [id, i]))
  let kept = 0
  ids.forEach((id, i) => {
    const j = old.get(id)
    if (j === undefined || j >= from.n) return
    sim.x[i] = from.x[j]!
    sim.y[i] = from.y[j]!
    kept++
  })
  return kept
}

/** Hareketsiz yerleşim (azaltılmış hareket, ilk çizim): soğuyana ya da `maxTicks`'e dek ekrana çizmeden ilerletir. */
export function settle(sim: Sim, maxTicks = 400): void {
  for (let i = 0; i < maxTicks && sim.alpha >= ALPHA_MIN; i++) tick(sim)
}

/** Sürükleme vb. sonrası yeniden ısıtma. */
export function reheat(sim: Sim, alpha = 0.3): void {
  sim.alpha = Math.max(sim.alpha, alpha)
}

// ---------------------------------------------------------------------------------------------
// Görünüm (yakınlaştırma / kaydırma): ekran = dünya * k + (tx, ty)

export interface View {
  tx: number
  ty: number
  k: number
}

export const MIN_ZOOM = 0.03
export const MAX_ZOOM = 8

export function toScreen(v: View, wx: number, wy: number): [number, number] {
  return [wx * v.k + v.tx, wy * v.k + v.ty]
}

export function toWorld(v: View, sx: number, sy: number): [number, number] {
  return [(sx - v.tx) / v.k, (sy - v.ty) / v.k]
}

/** İmlecin altındaki dünya noktası sabit kalacak şekilde yakınlaştırır. */
export function zoomAt(v: View, factor: number, sx: number, sy: number): View {
  const k = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.k * factor))
  const [wx, wy] = toWorld(v, sx, sy)
  return { k, tx: sx - wx * k, ty: sy - wy * k }
}

/** Seçili düğümleri (boşsa hepsini) `w × h` alana sığdırır. */
export function fitView(sim: Sim, w: number, h: number, only?: ReadonlyArray<number>, pad = 24): View {
  const ids = only && only.length > 0 ? only : Array.from({ length: sim.n }, (_, i) => i)
  if (ids.length === 0 || w <= 0 || h <= 0) return { tx: w / 2, ty: h / 2, k: 1 }
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const i of ids) {
    minX = Math.min(minX, sim.x[i]!)
    maxX = Math.max(maxX, sim.x[i]!)
    minY = Math.min(minY, sim.y[i]!)
    maxY = Math.max(maxY, sim.y[i]!)
  }
  const bw = Math.max(maxX - minX, 1)
  const bh = Math.max(maxY - minY, 1)
  const k = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.min((w - pad * 2) / bw, (h - pad * 2) / bh, 2)))
  return { k, tx: w / 2 - ((minX + maxX) / 2) * k, ty: h / 2 - ((minY + maxY) / 2) * k }
}

/** Dünya koordinatında `maxDist` içindeki en yakın görünür düğüm; yoksa -1. */
export function nearest(sim: Sim, wx: number, wy: number, maxDist: number, visible?: (i: number) => boolean): number {
  let best = -1
  let bestD = maxDist * maxDist
  for (let i = 0; i < sim.n; i++) {
    if (visible && !visible(i)) continue
    const dx = sim.x[i]! - wx
    const dy = sim.y[i]! - wy
    const d = dx * dx + dy * dy
    if (d <= bestD) {
      bestD = d
      best = i
    }
  }
  return best
}
