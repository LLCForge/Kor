// Kuvvet yönelimli graf çizimi (canvas 2D + layout.ts). Ekran okuyucular ve klavye için GraphList aynı veriyi sunar;
// bu tuval yalnızca görsel bir özettir (role="img") ama +/−/0/ok tuşlarıyla yakınlaştırılıp kaydırılabilir.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from 'react'
import type { Snapshot } from '../../../ipc-graph.ts'
import { ALPHA_MIN, adoptPositions, createSim, fitView, nearest, reheat, settle, tick, toScreen, toWorld, zoomAt, type Sim, type View } from './layout.ts'
import { labelled, radius } from './model.ts'

export interface Layers {
  /** dizin → olay zamanı (ms): ajan şu an burada */
  touchedNow: ReadonlyMap<number, number>
  touchedSession: ReadonlySet<number>
  changed: ReadonlySet<number>
  hotspots: ReadonlySet<number>
}

interface GraphCanvasProps {
  snapshot: Snapshot
  mask: Uint8Array
  layers: Layers
  selected: number | null
  pathFrom: number | null
  path: ReadonlyArray<[number, number]>
  onSelect: (index: number | null) => void
  reducedMotion: boolean
  /** Değişince görünüm (varsa) seçili düğüme, yoksa tüm grafa sığdırılır */
  fitSeq: number
}

const FLASH_MS = 2500

interface Palette {
  border: string
  muted: string
  text: string
  soft: string
  accent: string
  warn: string
  add: string
  bg: string
  font: string
}

function readPalette(el: Element): Palette {
  const cs = getComputedStyle(el)
  const v = (name: string) => cs.getPropertyValue(name).trim()
  return {
    border: v('--border'),
    muted: v('--muted'),
    text: v('--text'),
    soft: v('--text-soft') || v('--muted'),
    accent: v('--accent'),
    warn: v('--warn'),
    add: v('--add-text'),
    bg: v('--bg'),
    font: `${v('--kor-text-12') || 'small'} ${v('--font-sans') || 'sans-serif'}`,
  }
}

export function GraphCanvas({ snapshot, mask, layers, selected, pathFrom, path, onSelect, reducedMotion, fitSeq }: GraphCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const view = useRef<View>({ tx: 0, ty: 0, k: 1 })
  const touchedView = useRef(false) // kullanıcı yakınlaştırdı/kaydırdıysa otomatik sığdırma durur
  const hovered = useRef(-1)
  const frame = useRef(0)
  const palette = useRef<Palette | null>(null)

  const prev = useRef<{ sim: Sim; ids: string[] } | null>(null)
  const continued = useRef(false)
  const sim = useMemo<Sim>(() => {
    const s = createSim(
      snapshot.nodes.length,
      snapshot.edges.map(([a, b]) => [a, b] as const),
      snapshot.nodes.map((n) => n.community),
    )
    // Canlı güncellemede (dosya kaydı) düğümler yerinde kalsın: önceki konumlar kimliğe göre taşınır.
    const kept = prev.current ? adoptPositions(s, snapshot.nodes.map((n) => n.id), prev.current.sim, prev.current.ids) : 0
    continued.current = kept > s.n / 2
    if (continued.current) s.alpha = 0.3
    // Azaltılmış harekette yerleşim ekrana çizilmeden tamamlanır ve donar.
    if (reducedMotion) settle(s, 500)
    prev.current = { sim: s, ids: snapshot.nodes.map((n) => n.id) }
    return s
  }, [snapshot, reducedMotion])

  const label = useMemo(() => ({ few: labelled(snapshot.nodes, mask, 25), many: labelled(snapshot.nodes, mask, 160) }), [snapshot, mask])

  // Çizim durumu ref'te: rAF döngüsü her karede en güncelini okur.
  const state = useRef({ mask, layers, selected, pathFrom, path, label })
  state.current = { mask, layers, selected, pathFrom, path, label }

  const draw = () => {
    const canvas = canvasRef.current
    const ctx = canvas?.getContext('2d')
    if (!canvas || !ctx) return
    const p = (palette.current ??= readPalette(canvas))
    const dpr = window.devicePixelRatio || 1
    const { w, h } = { w: canvas.width / dpr, h: canvas.height / dpr }
    const v = view.current
    const { mask, layers, selected, pathFrom, path, label } = state.current
    const now = performance.now()
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.clearRect(0, 0, w, h)
    ctx.setTransform(v.k * dpr, 0, 0, v.k * dpr, v.tx * dpr, v.ty * dpr)

    // Kenarlar: tek yol, soluk
    ctx.lineWidth = 1 / v.k
    ctx.strokeStyle = p.border
    ctx.globalAlpha = 0.9
    ctx.beginPath()
    for (const [a, b] of snapshot.edges) {
      if (!mask[a] || !mask[b]) continue
      ctx.moveTo(sim.x[a]!, sim.y[a]!)
      ctx.lineTo(sim.x[b]!, sim.y[b]!)
    }
    ctx.stroke()
    ctx.globalAlpha = 1
    // Seçili düğümün kenarları
    if (selected !== null) {
      ctx.strokeStyle = p.soft
      ctx.lineWidth = 1.5 / v.k
      ctx.beginPath()
      for (const [a, b] of snapshot.edges) {
        if ((a !== selected && b !== selected) || !mask[a] || !mask[b]) continue
        ctx.moveTo(sim.x[a]!, sim.y[a]!)
        ctx.lineTo(sim.x[b]!, sim.y[b]!)
      }
      ctx.stroke()
    }
    // Yol
    if (path.length > 0) {
      ctx.strokeStyle = p.accent
      ctx.lineWidth = 3 / v.k
      ctx.beginPath()
      for (const [a, b] of path) {
        ctx.moveTo(sim.x[a]!, sim.y[a]!)
        ctx.lineTo(sim.x[b]!, sim.y[b]!)
      }
      ctx.stroke()
    }

    // Düğümler
    let flashing = false
    for (let i = 0; i < sim.n; i++) {
      if (!mask[i]) continue
      const n = snapshot.nodes[i]!
      const r = radius(n)
      const x = sim.x[i]!
      const y = sim.y[i]!
      const touchedAt = layers.touchedNow.get(i)
      const live = touchedAt !== undefined && now - touchedAt < FLASH_MS
      ctx.fillStyle = live ? p.accent : layers.changed.has(i) ? p.add : p.muted
      ctx.beginPath()
      if (n.kind === 'module' || n.kind === 'package') ctx.rect(x - r, y - r, r * 2, r * 2)
      else ctx.arc(x, y, r, 0, Math.PI * 2)
      ctx.fill()
      if (layers.hotspots.has(i)) ring(ctx, x, y, r + 2.5 / v.k, p.warn, 2 / v.k)
      if (layers.touchedSession.has(i) && !live) ring(ctx, x, y, r + 1.5 / v.k, p.accent, 1 / v.k)
      if (live) {
        flashing = true
        if (reducedMotion) ring(ctx, x, y, r + 4 / v.k, p.accent, 2 / v.k)
        else {
          const t = (now - touchedAt) / FLASH_MS
          ctx.globalAlpha = 1 - t
          ring(ctx, x, y, r + (4 + t * 18) / v.k, p.accent, 2 / v.k)
          ctx.globalAlpha = 1
        }
      }
      if (i === pathFrom) ring(ctx, x, y, r + 5 / v.k, p.accent, 2 / v.k, [3 / v.k, 3 / v.k])
      if (i === selected) ring(ctx, x, y, r + 3.5 / v.k, p.text, 2.5 / v.k)
    }

    // Etiketler ekran uzayında
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.font = p.font
    ctx.textBaseline = 'middle'
    const names = v.k >= 1.4 ? label.many : v.k >= 0.5 ? label.few : new Set<number>()
    const show = new Set(names)
    for (const i of [selected, pathFrom, hovered.current]) if (i !== null && i >= 0) show.add(i)
    for (const i of layers.touchedNow.keys()) show.add(i)
    for (const i of show) {
      if (!mask[i]) continue
      const n = snapshot.nodes[i]!
      const [sx, sy] = toScreen(v, sim.x[i]!, sim.y[i]!)
      if (sx < -200 || sy < -20 || sx > w + 20 || sy > h + 20) continue
      const tx = sx + radius(n) * v.k + 5
      ctx.lineWidth = 3
      ctx.strokeStyle = p.bg
      ctx.strokeText(n.name, tx, sy)
      ctx.fillStyle = i === selected || i === hovered.current ? p.text : p.soft
      ctx.fillText(n.name, tx, sy)
    }
    return flashing
  }

  // Tek rAF döngüsü: yerleşim sürüyor ya da yanıp sönen düğüm varken çalışır, sonra durur. Döngü her karede
  // en güncel çizimi `latest` üzerinden okur (veri değişince eski kapanış eski grafı çizmesin).
  const latest = useRef({ draw, refit: (_?: number[]) => {}, sim, reducedMotion })
  const kick = () => {
    if (frame.current) return
    const loop = () => {
      frame.current = 0
      const { draw, refit, sim, reducedMotion } = latest.current
      let moving = false
      if (!reducedMotion && sim.alpha >= ALPHA_MIN) {
        tick(sim)
        moving = true
        if (!touchedView.current && sim.alpha > 0.05) refit()
      }
      const flashing = draw() ?? false
      if (moving || flashing) frame.current = requestAnimationFrame(loop)
    }
    frame.current = requestAnimationFrame(loop)
  }

  const refit = (only?: number[]) => {
    const canvas = canvasRef.current
    if (!canvas) return
    const visible = Array.from({ length: sim.n }, (_, i) => i).filter((i) => state.current.mask[i])
    view.current = fitView(sim, size.w, size.h, only ?? visible)
    if (only) view.current = { ...view.current, k: Math.min(view.current.k, 1.6) }
  }
  latest.current = { draw, refit, sim, reducedMotion }

  // Sıfırlama şart: StrictMode efekti iki kez kurar; iptal edilen kare kimliği kalırsa `kick` bir daha çizmez.
  useEffect(
    () => () => {
      cancelAnimationFrame(frame.current)
      frame.current = 0
    },
    [],
  )

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const ro = new ResizeObserver(([entry]) => {
      if (!entry) return
      const { width, height } = entry.contentRect
      const dpr = window.devicePixelRatio || 1
      canvas.width = Math.max(1, Math.round(width * dpr))
      canvas.height = Math.max(1, Math.round(height * dpr))
      palette.current = null // tema değişmiş olabilir
      setSize({ w: width, h: height })
    })
    ro.observe(canvas)
    return () => ro.disconnect()
  }, [])

  // Yeni veri ya da boyut: görünümü sığdır ve çiz.
  useEffect(() => {
    if (!continued.current) touchedView.current = false
    if (!touchedView.current) refit()
    kick()
  }, [sim, size.w, size.h])

  useEffect(() => {
    if (fitSeq === 0) return
    refit(state.current.selected !== null ? [state.current.selected] : undefined)
    touchedView.current = true
    kick()
  }, [fitSeq])

  // Filtre/vurgu/seçim değişti: yeniden çiz (yanıp sönme varsa döngü sürer).
  useEffect(() => {
    kick()
  }, [mask, layers, selected, pathFrom, path, label])

  // Etkileşim
  const drag = useRef<{ mode: 'pan' | 'node'; index: number; sx: number; sy: number; tx: number; ty: number; moved: boolean } | null>(null)

  const local = (e: { clientX: number; clientY: number }): [number, number] => {
    const r = canvasRef.current!.getBoundingClientRect()
    return [e.clientX - r.left, e.clientY - r.top]
  }
  const hit = (sx: number, sy: number) => {
    const [wx, wy] = toWorld(view.current, sx, sy)
    return nearest(sim, wx, wy, 16 / view.current.k, (i) => state.current.mask[i] === 1)
  }

  const onPointerDown = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const [sx, sy] = local(e)
    const i = hit(sx, sy)
    e.currentTarget.setPointerCapture(e.pointerId)
    drag.current = { mode: i >= 0 ? 'node' : 'pan', index: i, sx, sy, tx: view.current.tx, ty: view.current.ty, moved: false }
  }
  const onPointerMove = (e: ReactPointerEvent<HTMLCanvasElement>) => {
    const [sx, sy] = local(e)
    const d = drag.current
    if (!d) {
      const i = hit(sx, sy)
      if (i !== hovered.current) {
        hovered.current = i
        e.currentTarget.style.cursor = i >= 0 ? 'pointer' : 'grab'
        kick()
      }
      return
    }
    if (!d.moved && Math.hypot(sx - d.sx, sy - d.sy) < 4) return
    d.moved = true
    touchedView.current = true
    if (d.mode === 'pan') {
      view.current = { ...view.current, tx: d.tx + sx - d.sx, ty: d.ty + sy - d.sy }
    } else {
      const [wx, wy] = toWorld(view.current, sx, sy)
      sim.pinned[d.index] = 1
      sim.x[d.index] = wx
      sim.y[d.index] = wy
      if (!reducedMotion) reheat(sim, 0.2)
    }
    kick()
  }
  const onPointerUp = () => {
    const d = drag.current
    drag.current = null
    if (!d) return
    if (d.mode === 'node') sim.pinned[d.index] = 0
    if (!d.moved) onSelect(d.index >= 0 ? d.index : null)
  }

  // Tekerlek: pasif olmayan dinleyici (sayfa kaymasın).
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const r = canvas.getBoundingClientRect()
      view.current = zoomAt(view.current, Math.exp(-e.deltaY * 0.0015), e.clientX - r.left, e.clientY - r.top)
      touchedView.current = true
      kick()
    }
    canvas.addEventListener('wheel', onWheel, { passive: false })
    return () => canvas.removeEventListener('wheel', onWheel)
  }, [sim])

  const onKeyDown = (e: ReactKeyboardEvent<HTMLCanvasElement>) => {
    const v = view.current
    const step = 60
    const next: Record<string, () => View> = {
      '+': () => zoomAt(v, 1.25, size.w / 2, size.h / 2),
      '=': () => zoomAt(v, 1.25, size.w / 2, size.h / 2),
      '-': () => zoomAt(v, 0.8, size.w / 2, size.h / 2),
      ArrowLeft: () => ({ ...v, tx: v.tx + step }),
      ArrowRight: () => ({ ...v, tx: v.tx - step }),
      ArrowUp: () => ({ ...v, ty: v.ty + step }),
      ArrowDown: () => ({ ...v, ty: v.ty - step }),
    }
    if (e.key === '0') {
      refit()
    } else if (e.key === 'Escape') {
      onSelect(null)
    } else if (next[e.key]) {
      view.current = next[e.key]!()
    } else {
      return
    }
    e.preventDefault()
    touchedView.current = true
    kick()
  }

  const visibleCount = useMemo(() => mask.reduce((a, b) => a + b, 0), [mask])

  return (
    <canvas
      ref={canvasRef}
      className="kg-canvas"
      role="img"
      tabIndex={0}
      aria-label={`Proje grafı: ${visibleCount} düğüm görünüyor. Yakınlaştırmak için artı ve eksi, kaydırmak için ok tuşları, sığdırmak için sıfır. Düğüm seçmek için liste görünümünü kullan.`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => (drag.current = null)}
      onPointerLeave={() => {
        if (hovered.current !== -1 && !drag.current) {
          hovered.current = -1
          kick()
        }
      }}
      onKeyDown={onKeyDown}
    />
  )
}

function ring(ctx: CanvasRenderingContext2D, x: number, y: number, r: number, color: string, width: number, dash?: number[]) {
  ctx.strokeStyle = color
  ctx.lineWidth = width
  if (dash) ctx.setLineDash(dash)
  ctx.beginPath()
  ctx.arc(x, y, r, 0, Math.PI * 2)
  ctx.stroke()
  if (dash) ctx.setLineDash([])
}
