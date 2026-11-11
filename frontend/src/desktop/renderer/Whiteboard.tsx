// [g20] Tuval ve beyaz tahta: serbest çizim (#129); kutu, ok ve yapışkan not (#130); Mermaid'e dışa aktarma (#131);
//      el çizimi (kroki) stili (#132); ızgaraya ve diğer nesnelere oturma kılavuzu (#133); PNG/SVG dışa aktarma
//      (#134, rich-export); anlık görüntü sürümleri (#135). Çizim yalnızca yereldir; ağ ve dosya erişimi yoktur.
import { Brush, Camera, Download, Grid3x3, ImageDown, MousePointer2, MoveRight, PenTool, Square, StickyNote, Trash, Undo2, Workflow } from 'lucide-react'
import { useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react'
import { errorMessage } from './api.ts'
import { CopyButton } from './CopyButton.tsx'
import {
  alignRect,
  BOARD_H,
  BOARD_W,
  boardToMermaid,
  boardToSvg,
  COLORS,
  GRID,
  hitTest,
  isFrame,
  MAX_PEN_POINTS,
  MAX_SHAPES,
  moveShape,
  shapeBounds,
  shapePrims,
  snap,
  type Board,
  type Guide,
  type Prim,
  type Pt,
  type Shape,
} from './g20-board.ts'
import { loadCurrentBoard, loadSnapshots, MAX_SNAPSHOTS, saveCurrentBoard, saveSnapshots, type Snapshot } from './g20-board-store.ts'
import { getRichBlock } from './rich-block-registry.ts'
import { downloadBlob, downloadText, svgToPng } from './rich-export.ts'
import { uiLocale } from './ui-locale.ts'

type Tool = 'select' | 'pen' | 'box' | 'arrow' | 'note'

const TOOLS: { id: Tool; label: string; icon: ReactNode }[] = [
  { id: 'select', label: 'Seç ve taşı', icon: <MousePointer2 size={15} aria-hidden="true" /> },
  { id: 'pen', label: 'Serbest çizim', icon: <PenTool size={15} aria-hidden="true" /> },
  { id: 'box', label: 'Kutu', icon: <Square size={15} aria-hidden="true" /> },
  { id: 'arrow', label: 'Ok', icon: <MoveRight size={15} aria-hidden="true" /> },
  { id: 'note', label: 'Yapışkan not', icon: <StickyNote size={15} aria-hidden="true" /> },
]

type Gesture = { kind: 'create'; start: Pt; shape: Shape } | { kind: 'move'; start: Pt; orig: Shape; shape: Shape }

// Kroki yolları tohumludur ama hesaplaması tekrar etmesin: şekil nesnesi başına önbellek.
const primCache = { rough: new WeakMap<Shape, Prim[]>(), plain: new WeakMap<Shape, Prim[]>() }
function primsOf(shape: Shape, rough: boolean): Prim[] {
  const cache = rough ? primCache.rough : primCache.plain
  let prims = cache.get(shape)
  if (!prims) {
    prims = shapePrims(shape, rough)
    cache.set(shape, prims)
  }
  return prims
}

function PrimEl({ p }: { p: Prim }) {
  if (p.tag === 'path') return <path d={p.d} stroke={p.stroke} fill={p.fill} strokeWidth={p.width} strokeLinecap="round" strokeLinejoin="round" />
  if (p.tag === 'rect') return <rect x={p.x} y={p.y} width={p.w} height={p.h} stroke={p.stroke} fill={p.fill} strokeWidth={p.width} rx={3} />
  return (
    <text x={p.x} y={p.y} fill={p.color} fontSize={p.size} fontFamily="system-ui, sans-serif" textAnchor="middle" dominantBaseline="middle">
      {p.lines.map((line, i) => (
        <tspan key={i} x={p.x} dy={i === 0 ? 0 : p.size * 1.25}>
          {line}
        </tspan>
      ))}
    </text>
  )
}

const snapPt = (p: Pt): Pt => ({ x: snap(p.x), y: snap(p.y) })

export function Whiteboard() {
  const [board, setBoard] = useState<Board>(loadCurrentBoard)
  const [undo, setUndo] = useState<Board[]>([])
  const [tool, setTool] = useState<Tool>('select')
  const [rough, setRough] = useState(true)
  const [useGrid, setUseGrid] = useState(true)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [live, setLive] = useState<Shape | null>(null)
  const [guides, setGuides] = useState<Guide[]>([])
  const [snapshots, setSnapshots] = useState<Snapshot[]>(loadSnapshots)
  const [status, setStatus] = useState<string | null>(null)
  const [mermaid, setMermaid] = useState<string | null>(null)
  const svgRef = useRef<SVGSVGElement>(null)
  const gesture = useRef<Gesture | null>(null)

  const persist = (next: Board) => {
    if (!saveCurrentBoard(next)) setStatus('Tuval bu cihazda kaydedilemedi; yalnızca bu oturumda duruyor.')
  }
  const commit = (next: Board) => {
    setUndo((u) => [...u.slice(-49), board])
    setBoard(next)
    persist(next)
  }
  const undoLast = () => {
    const prev = undo.at(-1)
    if (!prev) return
    setUndo((u) => u.slice(0, -1))
    setBoard(prev)
    persist(prev)
  }

  const shapes = live ? (board.shapes.some((s) => s.id === live.id) ? board.shapes.map((s) => (s.id === live.id ? live : s)) : [...board.shapes, live]) : board.shapes
  const selected = board.shapes.find((s) => s.id === selectedId) ?? null

  const toBoard = (e: PointerEvent<SVGSVGElement>): Pt => {
    const ctm = svgRef.current?.getScreenCTM()
    if (!ctm) return { x: 0, y: 0 }
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(ctm.inverse())
    return { x: Math.max(0, Math.min(BOARD_W, p.x)), y: Math.max(0, Math.min(BOARD_H, p.y)) }
  }

  const onPointerDown = (e: PointerEvent<SVGSVGElement>) => {
    if (e.button !== 0) return
    const p = toBoard(e)
    e.currentTarget.setPointerCapture(e.pointerId)
    if (tool === 'select') {
      const hit = hitTest(board.shapes, p)
      setSelectedId(hit?.id ?? null)
      gesture.current = hit ? { kind: 'move', start: p, orig: hit, shape: hit } : null
      return
    }
    if (board.shapes.length >= MAX_SHAPES) {
      setStatus(`Tuvalde en fazla ${MAX_SHAPES} öğe olabilir.`)
      return
    }
    const start = useGrid && tool !== 'pen' ? snapPt(p) : p
    const id = crypto.randomUUID()
    const shape: Shape =
      tool === 'pen' ? { id, kind: 'pen', points: [p] } : tool === 'arrow' ? { id, kind: 'arrow', from: start, to: start } : { id, kind: tool, x: start.x, y: start.y, w: 0, h: 0, text: '' }
    gesture.current = { kind: 'create', start, shape }
    setLive(shape)
  }

  const onPointerMove = (e: PointerEvent<SVGSVGElement>) => {
    const g = gesture.current
    if (!g) return
    const p = toBoard(e)
    let next: Shape
    if (g.kind === 'move') {
      next = moveShape(g.orig, p.x - g.start.x, p.y - g.start.y)
      if (isFrame(next)) {
        const others = board.shapes.filter((s) => s.id !== g.orig.id && isFrame(s)).map(shapeBounds)
        const aligned = alignRect(shapeBounds(next), others, useGrid)
        next = { ...next, x: aligned.x, y: aligned.y }
        setGuides(aligned.guides)
      } else if (useGrid) {
        const b = shapeBounds(g.orig)
        next = moveShape(g.orig, snap(b.x + p.x - g.start.x) - b.x, snap(b.y + p.y - g.start.y) - b.y)
      }
    } else if (g.shape.kind === 'pen') {
      const last = g.shape.points.at(-1)
      if ((last && Math.hypot(p.x - last.x, p.y - last.y) < 2) || g.shape.points.length >= MAX_PEN_POINTS) return
      next = { ...g.shape, points: [...g.shape.points, p] }
    } else if (g.shape.kind === 'arrow') {
      next = { ...g.shape, to: useGrid ? snapPt(p) : p }
    } else {
      const end = useGrid ? snapPt(p) : p
      next = { ...g.shape, x: Math.min(g.start.x, end.x), y: Math.min(g.start.y, end.y), w: Math.abs(end.x - g.start.x), h: Math.abs(end.y - g.start.y) }
    }
    g.shape = next
    setLive(next)
  }

  const finishGesture = (cancelled: boolean) => {
    const g = gesture.current
    gesture.current = null
    setGuides([])
    setLive(null)
    if (!g || cancelled) return
    if (g.kind === 'move') {
      if (g.shape !== g.orig) commit({ shapes: board.shapes.map((s) => (s.id === g.orig.id ? g.shape : s)) })
      return
    }
    let shape = g.shape
    if (shape.kind === 'pen' && shape.points.length < 2) return
    if (shape.kind === 'arrow' && Math.hypot(shape.to.x - shape.from.x, shape.to.y - shape.from.y) < 10) return
    // Tıklayıp bırakınca varsayılan boyut.
    if (isFrame(shape) && shape.w < 10 && shape.h < 10) shape = { ...shape, w: 160, h: shape.kind === 'note' ? 120 : 80 }
    commit({ shapes: [...board.shapes, shape] })
    if (isFrame(shape)) setSelectedId(shape.id)
  }

  const updateSelectedText = (text: string) => {
    if (!selected || !isFrame(selected)) return
    const next = { shapes: board.shapes.map((s) => (s.id === selected.id ? { ...selected, text: text.slice(0, 500) } : s)) }
    setBoard(next)
    persist(next)
  }

  const onKeyDown = (e: KeyboardEvent<SVGSVGElement>) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') {
      e.preventDefault()
      undoLast()
      return
    }
    if (!selected) return
    if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault()
      commit({ shapes: board.shapes.filter((s) => s.id !== selected.id) })
      setSelectedId(null)
    } else if (e.key === 'Escape') {
      e.preventDefault() // diyalog kapanmasın; önce seçim bırakılır
      setSelectedId(null)
    } else if (e.key.startsWith('Arrow')) {
      e.preventDefault()
      const step = e.shiftKey ? 1 : GRID
      const dx = e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0
      const dy = e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0
      commit({ shapes: board.shapes.map((s) => (s.id === selected.id ? moveShape(s, dx, dy) : s)) })
    }
  }

  const takeSnapshot = () => {
    const next = [{ id: crypto.randomUUID(), at: Date.now(), board }, ...snapshots].slice(0, MAX_SNAPSHOTS)
    setSnapshots(next)
    setStatus(saveSnapshots(next) ? 'Anlık görüntü kaydedildi.' : 'Anlık görüntü yalnızca bu oturumda tutuluyor (yerel depolama kullanılamıyor).')
  }
  const removeSnapshot = (id: string) => {
    const next = snapshots.filter((s) => s.id !== id)
    setSnapshots(next)
    if (!saveSnapshots(next)) setStatus('Sürüm listesi kaydedilemedi.')
  }

  const exportPng = async () => {
    try {
      downloadBlob(await svgToPng(boardToSvg(board, rough)), 'tuval.png')
    } catch (err) {
      setStatus(`PNG oluşturulamadı: ${errorMessage(err)}`)
    }
  }

  const Mermaid = getRichBlock('mermaid')
  const selectedBounds = selected ? shapeBounds(live?.id === selected.id ? live : selected) : null

  return (
    <div className="g20-wb">
      <div className="g20-wb-toolbar" role="toolbar" aria-label="Tuval araçları">
        <div className="g20-wb-group">
          {TOOLS.map((t) => (
            <button key={t.id} type="button" className="action-btn" aria-pressed={tool === t.id} aria-label={t.label} title={t.label} onClick={() => setTool(t.id)}>
              {t.icon}
            </button>
          ))}
        </div>
        <div className="g20-wb-group">
          <button type="button" className="action-btn" aria-pressed={rough} aria-label="El çizimi stili" title="El çizimi (kroki) stili" onClick={() => setRough((v) => !v)}>
            <Brush size={15} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" aria-pressed={useGrid} aria-label="Izgaraya oturt" title="Izgaraya ve nesnelere oturt" onClick={() => setUseGrid((v) => !v)}>
            <Grid3x3 size={15} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" aria-label="Geri al (Ctrl+Z)" title="Geri al (Ctrl+Z)" disabled={undo.length === 0} onClick={undoLast}>
            <Undo2 size={15} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" aria-label="Tuvali temizle" title="Tuvali temizle (geri alınabilir)" disabled={board.shapes.length === 0} onClick={() => commit({ shapes: [] })}>
            <Trash size={15} aria-hidden="true" />
          </button>
        </div>
        <div className="g20-wb-group">
          <button type="button" className="action-btn" aria-label="Mermaid'e aktar" title="Mermaid diyagramına aktar" onClick={() => setMermaid(boardToMermaid(board))}>
            <Workflow size={15} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" aria-label="SVG olarak indir" title="SVG olarak indir" onClick={() => downloadText(boardToSvg(board, rough), 'tuval.svg', 'image/svg+xml')}>
            <Download size={15} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" aria-label="PNG olarak indir" title="PNG olarak indir" onClick={() => void exportPng()}>
            <ImageDown size={15} aria-hidden="true" />
          </button>
          <button type="button" className="action-btn" aria-label="Anlık görüntü al" title="Anlık görüntü al (sürüm)" onClick={takeSnapshot}>
            <Camera size={15} aria-hidden="true" />
          </button>
        </div>
        {selected && isFrame(selected) && (
          <label className="g20-wb-text">
            <span>Metin</span>
            <textarea key={selected.id} rows={2} value={selected.text} onChange={(e) => updateSelectedText(e.target.value)} maxLength={500} autoFocus />
          </label>
        )}
      </div>
      <p className="g20-note" id="g20-wb-help">
        Araç seçip sürükleyerek çizin. Seç aracıyla öğeleri taşıyın; Delete siler, Ctrl+Z geri alır, ok tuşları seçili öğeyi ızgara adımıyla taşır.
      </p>
      {status && (
        <p className="g20-note" role="status">
          {status}
        </p>
      )}
      <svg
        ref={svgRef}
        className="g20-wb-canvas"
        data-tool={tool}
        viewBox={`0 0 ${BOARD_W} ${BOARD_H}`}
        role="application"
        aria-label="Beyaz tahta çizim alanı"
        aria-describedby="g20-wb-help"
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={() => finishGesture(false)}
        onPointerCancel={() => finishGesture(true)}
        onKeyDown={onKeyDown}
      >
        <rect width={BOARD_W} height={BOARD_H} fill={COLORS.paper} />
        {useGrid && (
          <>
            <defs>
              <pattern id="g20-grid" width={GRID} height={GRID} patternUnits="userSpaceOnUse">
                <path d={`M${GRID} 0 L0 0 0 ${GRID}`} fill="none" stroke={COLORS.grid} strokeWidth={1} />
              </pattern>
            </defs>
            <rect width={BOARD_W} height={BOARD_H} fill="url(#g20-grid)" />
          </>
        )}
        {shapes.map((s) => (
          <g key={s.id}>
            {primsOf(s, rough).map((p, i) => (
              <PrimEl key={i} p={p} />
            ))}
          </g>
        ))}
        {selectedBounds && (
          <rect
            x={selectedBounds.x - 6}
            y={selectedBounds.y - 6}
            width={selectedBounds.w + 12}
            height={selectedBounds.h + 12}
            fill="none"
            stroke={COLORS.accent}
            strokeWidth={1.5}
            strokeDasharray="6 4"
            pointerEvents="none"
          />
        )}
        {guides.map((g, i) =>
          g.axis === 'x' ? (
            <line key={i} x1={g.at} x2={g.at} y1={0} y2={BOARD_H} stroke={COLORS.accent} strokeWidth={1} pointerEvents="none" />
          ) : (
            <line key={i} x1={0} x2={BOARD_W} y1={g.at} y2={g.at} stroke={COLORS.accent} strokeWidth={1} pointerEvents="none" />
          ),
        )}
      </svg>
      {mermaid !== null && (
        <section className="g20-wb-mermaid" aria-label="Mermaid çıktısı">
          <div className="g20-wb-group">
            <strong>Mermaid</strong>
            <CopyButton className="action-btn" label="Mermaid metnini kopyala" getText={() => mermaid} />
            <button type="button" className="btn small ghost" onClick={() => setMermaid(null)}>
              Kapat
            </button>
          </div>
          {Mermaid ? <Mermaid code={mermaid} language="mermaid" /> : <pre>{mermaid}</pre>}
        </section>
      )}
      <details className="g20-history">
        <summary>Sürümler ({snapshots.length})</summary>
        {snapshots.length === 0 ? (
          <p className="g20-note">Henüz anlık görüntü yok.</p>
        ) : (
          <ul>
            {snapshots.map((s) => (
              <li key={s.id} className="g20-wb-snapshot">
                <span>
                  {new Date(s.at).toLocaleString(uiLocale())} · {s.board.shapes.length} öğe
                </span>
                <button type="button" className="btn small" onClick={() => commit(s.board)}>
                  Geri yükle
                </button>
                <button type="button" className="btn small ghost" onClick={() => removeSnapshot(s.id)}>
                  Sil
                </button>
              </li>
            ))}
          </ul>
        )}
      </details>
    </div>
  )
}
