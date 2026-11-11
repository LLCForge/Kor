// [g20] Beyaz tahta modeli (saf; DOM yok): şekiller, ızgaraya/nesnelere oturma, el çizimi (kroki) yolları, ortak
//      çizim ilkelleri (canlı görünüm ve SVG dışa aktarma aynı ilkelleri kullanır), Mermaid'e dışa aktarma.
//      El çizimi için kütüphane yerine tohumlu küçük bir pürüz algoritması kullanılır (roughjs benzeri; bağımlılık yok).

export interface Pt {
  x: number
  y: number
}

export interface Rect {
  x: number
  y: number
  w: number
  h: number
}

export interface FrameShape extends Rect {
  id: string
  kind: 'box' | 'note'
  text: string
}

export interface ArrowShape {
  id: string
  kind: 'arrow'
  from: Pt
  to: Pt
}

export interface PenShape {
  id: string
  kind: 'pen'
  points: Pt[]
}

export type Shape = FrameShape | ArrowShape | PenShape

export interface Board {
  shapes: Shape[]
}

export const BOARD_W = 1200
export const BOARD_H = 700
export const GRID = 20
export const MAX_SHAPES = 2000
export const MAX_PEN_POINTS = 2000
export const COLORS = { paper: '#fbfaf7', ink: '#26241f', note: '#fde68a', noteInk: '#4a3b0b', grid: '#e7e3da', accent: '#c96442' } as const

export const snap = (v: number, grid = GRID) => Math.round(v / grid) * grid

export const isFrame = (s: Shape): s is FrameShape => s.kind === 'box' || s.kind === 'note'

export function shapeBounds(s: Shape): Rect {
  if (isFrame(s)) return { x: s.x, y: s.y, w: s.w, h: s.h }
  const pts = s.kind === 'arrow' ? [s.from, s.to] : s.points
  const xs = pts.map((p) => p.x)
  const ys = pts.map((p) => p.y)
  const x = Math.min(...xs)
  const y = Math.min(...ys)
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y }
}

export function moveShape(s: Shape, dx: number, dy: number): Shape {
  if (isFrame(s)) return { ...s, x: s.x + dx, y: s.y + dy }
  if (s.kind === 'arrow') return { ...s, from: { x: s.from.x + dx, y: s.from.y + dy }, to: { x: s.to.x + dx, y: s.to.y + dy } }
  return { ...s, points: s.points.map((p) => ({ x: p.x + dx, y: p.y + dy })) }
}

/** Noktaya en üstte duran şekil (son çizilen önde); kalem ve ok için çizgiye yakınlık kullanılır. */
export function hitTest(shapes: readonly Shape[], p: Pt, tolerance = 6): Shape | null {
  for (let i = shapes.length - 1; i >= 0; i--) {
    const s = shapes[i]
    if (!s) continue
    if (isFrame(s)) {
      if (p.x >= s.x - tolerance && p.x <= s.x + s.w + tolerance && p.y >= s.y - tolerance && p.y <= s.y + s.h + tolerance) return s
    } else {
      const pts = s.kind === 'arrow' ? [s.from, s.to] : s.points
      for (let j = 1; j < pts.length; j++) {
        const a = pts[j - 1]
        const b = pts[j]
        if (a && b && segmentDistance(p, a, b) <= tolerance) return s
      }
    }
  }
  return null
}

function segmentDistance(p: Pt, a: Pt, b: Pt): number {
  const dx = b.x - a.x
  const dy = b.y - a.y
  const len = dx * dx + dy * dy
  const t = len === 0 ? 0 : Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len))
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy))
}

export interface Guide {
  axis: 'x' | 'y'
  at: number
}

/**
 * Taşınan dikdörtgeni diğer nesnelerin sol/orta/sağ (ve üst/orta/alt) çizgilerine oturtur. Eşik içinde en yakın
 * hizalama uygulanır ve kılavuz çizgisi döner; eşleşme yoksa (ızgara açıksa) ızgaraya oturur.
 */
export function alignRect(moving: Rect, others: readonly Rect[], useGrid: boolean, threshold = 6): { x: number; y: number; guides: Guide[] } {
  const guides: Guide[] = []
  const axis = (pos: number, size: number, lines: number[], name: 'x' | 'y'): number => {
    let best: { delta: number; at: number } | null = null
    for (const offset of [0, size / 2, size]) {
      for (const line of lines) {
        const delta = line - (pos + offset)
        if (Math.abs(delta) <= threshold && (!best || Math.abs(delta) < Math.abs(best.delta))) best = { delta, at: line }
      }
    }
    if (best) {
      guides.push({ axis: name, at: best.at })
      return pos + best.delta
    }
    return useGrid ? snap(pos) : pos
  }
  const xs = others.flatMap((o) => [o.x, o.x + o.w / 2, o.x + o.w])
  const ys = others.flatMap((o) => [o.y, o.y + o.h / 2, o.y + o.h])
  return { x: axis(moving.x, moving.w, xs, 'x'), y: axis(moving.y, moving.h, ys, 'y'), guides }
}

/** Şekil kimliğinden kararlı tohum: kroki her çizimde aynı görünür. */
export function seedOf(id: string): number {
  let h = 2166136261
  for (let i = 0; i < id.length; i++) h = Math.imul(h ^ id.charCodeAt(i), 16777619)
  return h >>> 0
}

function rng(seed: number): () => number {
  let a = seed || 1
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const r1 = (n: number) => Math.round(n * 10) / 10

/** İki kez, hafif kaydırılmış ve bükülmüş çizilen kroki çizgisi (SVG path d). */
export function roughLine(a: Pt, b: Pt, seed: number, roughness = 1.6): string {
  const rand = rng(seed)
  const len = Math.hypot(b.x - a.x, b.y - a.y)
  const amp = Math.min(roughness * 2, 0.5 + len / 60) * roughness
  const off = () => (rand() - 0.5) * 2 * amp
  const parts: string[] = []
  for (let pass = 0; pass < 2; pass++) {
    const mx = (a.x + b.x) / 2 + off()
    const my = (a.y + b.y) / 2 + off()
    parts.push(`M${r1(a.x + off())} ${r1(a.y + off())} Q${r1(mx)} ${r1(my)} ${r1(b.x + off())} ${r1(b.y + off())}`)
  }
  return parts.join(' ')
}

export function roughRect(r: Rect, seed: number): string {
  const p = [
    { x: r.x, y: r.y },
    { x: r.x + r.w, y: r.y },
    { x: r.x + r.w, y: r.y + r.h },
    { x: r.x, y: r.y + r.h },
  ] as const
  return [0, 1, 2, 3].map((i) => roughLine(p[i] as Pt, p[(i + 1) % 4] as Pt, seed + i)).join(' ')
}

export function penPath(points: readonly Pt[]): string {
  if (points.length === 0) return ''
  const [first, ...rest] = points
  return `M${r1(first?.x ?? 0)} ${r1(first?.y ?? 0)}${rest.map((p) => ` L${r1(p.x)} ${r1(p.y)}`).join('')}`
}

/** Ok ucu: hedefte iki kısa kanat. */
function arrowHead(from: Pt, to: Pt, size = 12): [Pt, Pt] {
  const angle = Math.atan2(to.y - from.y, to.x - from.x)
  const wing = (delta: number) => ({ x: to.x - size * Math.cos(angle + delta), y: to.y - size * Math.sin(angle + delta) })
  return [wing(Math.PI / 7), wing(-Math.PI / 7)]
}

export type Prim =
  | { tag: 'path'; d: string; stroke: string; fill: string; width: number }
  | { tag: 'rect'; x: number; y: number; w: number; h: number; stroke: string; fill: string; width: number }
  | { tag: 'text'; x: number; y: number; lines: string[]; color: string; size: number }

/** Şeklin çizim ilkelleri: canlı görünüm ve SVG dışa aktarma bunları paylaşır. */
export function shapePrims(s: Shape, rough: boolean): Prim[] {
  const seed = seedOf(s.id)
  if (s.kind === 'pen') return [{ tag: 'path', d: penPath(s.points), stroke: COLORS.ink, fill: 'none', width: 2 }]
  if (s.kind === 'arrow') {
    const [w1, w2] = arrowHead(s.from, s.to)
    const d = rough
      ? `${roughLine(s.from, s.to, seed)} ${roughLine(w1, s.to, seed + 7)} ${roughLine(w2, s.to, seed + 9)}`
      : `M${r1(s.from.x)} ${r1(s.from.y)} L${r1(s.to.x)} ${r1(s.to.y)} M${r1(w1.x)} ${r1(w1.y)} L${r1(s.to.x)} ${r1(s.to.y)} L${r1(w2.x)} ${r1(w2.y)}`
    return [{ tag: 'path', d, stroke: COLORS.ink, fill: 'none', width: 2 }]
  }
  const note = s.kind === 'note'
  const fill = note ? COLORS.note : COLORS.paper
  const body: Prim[] = rough
    ? [
        { tag: 'rect', x: s.x, y: s.y, w: s.w, h: s.h, stroke: 'none', fill, width: 0 },
        { tag: 'path', d: roughRect(s, seed), stroke: note ? COLORS.noteInk : COLORS.ink, fill: 'none', width: 1.6 },
      ]
    : [{ tag: 'rect', x: s.x, y: s.y, w: s.w, h: s.h, stroke: note ? COLORS.noteInk : COLORS.ink, fill, width: 1.6 }]
  const lines = s.text.split('\n').slice(0, 12)
  const size = 15
  const top = s.y + s.h / 2 - ((lines.length - 1) * size * 1.25) / 2
  return [...body, { tag: 'text', x: s.x + s.w / 2, y: top, lines, color: note ? COLORS.noteInk : COLORS.ink, size }]
}

const esc = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')

function primToSvg(p: Prim): string {
  if (p.tag === 'path') return `<path d="${p.d}" stroke="${p.stroke}" fill="${p.fill}" stroke-width="${p.width}" stroke-linecap="round" stroke-linejoin="round"/>`
  if (p.tag === 'rect') return `<rect x="${r1(p.x)}" y="${r1(p.y)}" width="${r1(p.w)}" height="${r1(p.h)}" stroke="${p.stroke}" fill="${p.fill}" stroke-width="${p.width}" rx="3"/>`
  const spans = p.lines.map((line, i) => `<tspan x="${r1(p.x)}" dy="${i === 0 ? 0 : r1(p.size * 1.25)}">${esc(line)}</tspan>`).join('')
  return `<text x="${r1(p.x)}" y="${r1(p.y)}" fill="${p.color}" font-size="${p.size}" font-family="system-ui, sans-serif" text-anchor="middle" dominant-baseline="middle">${spans}</text>`
}

/** Bağımsız SVG belgesi (dış referans yok; PNG'ye çevrilebilir). Metin kaçışlıdır. */
export function boardToSvg(board: Board, rough: boolean): string {
  const body = board.shapes.flatMap((s) => shapePrims(s, rough)).map(primToSvg).join('')
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${BOARD_W}" height="${BOARD_H}" viewBox="0 0 ${BOARD_W} ${BOARD_H}"><rect width="${BOARD_W}" height="${BOARD_H}" fill="${COLORS.paper}"/>${body}</svg>`
}

const mermaidLabel = (text: string, fallback: string) => {
  const clean = text.trim() || fallback
  return clean.replace(/"/g, '#quot;').replace(/\n/g, '<br/>')
}

/** Okun ucuna (kenar payı dahil) en yakın kutu/not. */
function boxAt(boxes: readonly (Rect & { id: string })[], p: Pt, margin = 24): string | null {
  let best: { id: string; d: number } | null = null
  for (const b of boxes) {
    const dx = Math.max(b.x - p.x, 0, p.x - (b.x + b.w))
    const dy = Math.max(b.y - p.y, 0, p.y - (b.y + b.h))
    const d = Math.hypot(dx, dy)
    if (d <= margin && (!best || d < best.d)) best = { id: b.id, d }
  }
  return best?.id ?? null
}

/** Kutular ve notlar düğüm, kutudan kutuya oklar kenar olur; serbest çizim dışarıda kalır. */
export function boardToMermaid(board: Board): string {
  const boxes = board.shapes.filter(isFrame)
  const ids = new Map(boxes.map((b, i) => [b.id, `n${i + 1}`]))
  const lines = ['flowchart LR']
  boxes.forEach((b, i) => {
    const label = mermaidLabel(b.text, b.kind === 'note' ? `Not ${i + 1}` : `Kutu ${i + 1}`)
    lines.push(b.kind === 'note' ? `  ${ids.get(b.id)}("${label}")` : `  ${ids.get(b.id)}["${label}"]`)
  })
  for (const s of board.shapes) {
    if (s.kind !== 'arrow') continue
    const from = boxAt(boxes, s.from)
    const to = boxAt(boxes, s.to)
    if (from && to && from !== to) lines.push(`  ${ids.get(from)} --> ${ids.get(to)}`)
  }
  return lines.join('\n')
}

const num = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)
const pt = (v: unknown): v is Pt => v !== null && typeof v === 'object' && num((v as Pt).x) && num((v as Pt).y)

/** Depodan okunan tahtayı doğrular (bozuk/eski kayıt güvenle elenir). */
export function parseBoard(value: unknown): Board {
  const raw = value !== null && typeof value === 'object' ? (value as { shapes?: unknown }).shapes : undefined
  if (!Array.isArray(raw)) return { shapes: [] }
  const shapes = raw.slice(0, MAX_SHAPES).flatMap((v): Shape[] => {
    const s = v as Record<string, unknown> | null
    if (!s || typeof s.id !== 'string') return []
    if ((s.kind === 'box' || s.kind === 'note') && num(s.x) && num(s.y) && num(s.w) && num(s.h)) {
      return [{ id: s.id, kind: s.kind, x: s.x, y: s.y, w: s.w, h: s.h, text: typeof s.text === 'string' ? s.text.slice(0, 500) : '' }]
    }
    if (s.kind === 'arrow' && pt(s.from) && pt(s.to)) return [{ id: s.id, kind: 'arrow', from: { x: s.from.x, y: s.from.y }, to: { x: s.to.x, y: s.to.y } }]
    if (s.kind === 'pen' && Array.isArray(s.points)) {
      const points = s.points.filter(pt).slice(0, MAX_PEN_POINTS).map((p) => ({ x: p.x, y: p.y }))
      return points.length > 1 ? [{ id: s.id, kind: 'pen', points }] : []
    }
    return []
  })
  return { shapes }
}
