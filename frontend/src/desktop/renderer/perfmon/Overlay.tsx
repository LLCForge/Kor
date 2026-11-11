// §32.3 (ADR 0062): gizli performans bindirmesi, Ctrl+Shift+P (macOS: Cmd+Shift+P). Açıkken saniyede bir
// tek durum güncellemesi: kare hızı (rAF sayacı), en uzun kare, IPC gecikmesi (kor-bridge sayaçları),
// olay hızı, süreç ağacının belleği (WebView dahil) ve tokio işçi/görev sayıları. Kapalıyken hiçbir döngü çalışmaz.
import { useEffect, useState } from 'react'
import { X } from 'lucide-react'
import { perfmonApi } from './api.ts'
import { snapshot, type IpcSnapshot } from './stats.ts'
import { isToggle } from './util.ts'
import type { PerfmonApi } from '../../ipc-perfmon.ts'
import './perfmon.css'

type Stats = Awaited<ReturnType<PerfmonApi['stats']>>
type Cause = Awaited<ReturnType<PerfmonApi['causes']>>[number]

interface View {
  fps: number
  worstFrame: number
  ipc: IpcSnapshot
  stats: Stats | null
  jsHeapMb: number | null
}

/** Chromium'un standart dışı yığın bilgisi (WebView2); diğer motorlarda yok. */
function jsHeapMb(): number | null {
  const mem = (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory
  return mem ? mem.usedJSHeapSize / 1048576 : null
}

const fmt = (n: number | null | undefined, digits = 1): string => (n === null || n === undefined ? '–' : n.toFixed(digits))

/** perf.log'daki en yavaş nedenler (`kor doctor --perf`); açılışta bir kez okunur. */
function Causes() {
  const [causes, setCauses] = useState<Cause[] | null>(null)
  useEffect(() => {
    let live = true
    perfmonApi.causes().then(
      (c) => live && setCauses(c),
      () => live && setCauses([]),
    )
    return () => {
      live = false
    }
  }, [])
  if (causes === null || causes.length === 0) return null
  return (
    <ol className="kpm-causes" aria-label="En yavaş nedenler" data-kor-cmd="perfmon.causes">
      {causes.slice(0, 3).map((c) => (
        <li key={`${c.source}/${c.kind}/${c.what}`} translate="no">
          {c.what} · {c.count}× · p95 {Math.round(c.p95_ms)} ms
        </li>
      ))}
    </ol>
  )
}

function Body({ onClose }: { onClose: () => void }) {
  const [view, setView] = useState<View | null>(null)
  useEffect(() => {
    let frames = 0
    let worst = 0
    let last = performance.now()
    let windowStart = last
    let raf = 0
    let timer = 0
    let stopped = false
    let stats: Stats | null = null
    const tick = (now: number) => {
      frames++
      worst = Math.max(worst, now - last)
      last = now
      if (!stopped) raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    const publish = () => {
      const now = performance.now()
      const secs = (now - windowStart) / 1000
      setView({ fps: secs > 0 ? frames / secs : 0, worstFrame: worst, ipc: snapshot(now), stats, jsHeapMb: jsHeapMb() })
      frames = 0
      worst = 0
      windowStart = now
      perfmonApi.stats().then(
        (s) => {
          stats = s
        },
        () => {
          stats = null
        },
      )
      if (!stopped) timer = window.setTimeout(publish, 1000)
    }
    timer = window.setTimeout(publish, 250)
    return () => {
      stopped = true
      cancelAnimationFrame(raf)
      clearTimeout(timer)
    }
  }, [])
  const s = view?.stats
  return (
    <section className="kpm-overlay" role="status" aria-label="Performans" data-kor-cmd="perfmon.stats">
      <header>
        <span>Performans</span>
        <button type="button" aria-label="Kapat" title="Kapat (Ctrl+Shift+P)" onClick={onClose}>
          <X size={14} />
        </button>
      </header>
      <dl>
        <dt>FPS</dt>
        <dd className={view && view.fps < 50 ? 'kpm-bad' : undefined}>{fmt(view?.fps, 0)}</dd>
        <dt>En uzun kare</dt>
        <dd className={view && view.worstFrame > 50 ? 'kpm-bad' : undefined}>{fmt(view?.worstFrame)} ms</dd>
        <dt>IPC p50 / p95</dt>
        <dd>
          {fmt(view?.ipc.p50)} / {fmt(view?.ipc.p95)} ms
        </dd>
        <dt>Son çağrı</dt>
        <dd translate="no">{view?.ipc.last ? `${view.ipc.last.method} ${fmt(view.ipc.last.ms)} ms` : '–'}</dd>
        <dt>Olay / sn</dt>
        <dd>{fmt(view?.ipc.eventsPerSec)}</dd>
        <dt>Bellek</dt>
        <dd>{fmt(s?.rssMb, 0)} MB</dd>
        <dt>JS yığını</dt>
        <dd>{fmt(view?.jsHeapMb, 0)} MB</dd>
        <dt>İşçi / görev</dt>
        <dd>
          {s?.workers ?? '–'} / {s?.tasks ?? '–'}
        </dd>
        <dt>Süreç</dt>
        <dd>{s?.processes ?? '–'}</dd>
      </dl>
      <Causes />
    </section>
  )
}

export function PerfOverlay() {
  const [open, setOpen] = useState(false)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isToggle(e)) return
      e.preventDefault()
      e.stopPropagation()
      setOpen((o) => !o)
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [])
  return open ? <Body onClose={() => setOpen(false)} /> : null
}
