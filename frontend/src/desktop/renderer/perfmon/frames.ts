// §32.3 (ADR 0062): 50 ms'yi aşan kareler ve nedenleri perf.log'a. Olay güdümlü (PerformanceObserver);
// zamanlayıcı ya da yoklama yok. Chromium/WebView2'de `long-animation-frame` betik düzeyinde neden verir,
// diğer motorlarda `longtask` (yalnızca süre) kullanılır.
import { SLOW_MS, reportSlow } from './stats.ts'

/** `long-animation-frame` girdisinin kullandığımız alanları (lib.dom henüz tanımlamıyor). */
interface LoafScript {
  duration: number
  invoker?: string
  sourceFunctionName?: string
  sourceURL?: string
  sourceCharPosition?: number
}

interface LoafEntry extends PerformanceEntry {
  scripts?: LoafScript[]
  blockingDuration?: number
}

/** En uzun betikten okunabilir neden: `invoker` > işlev adı > dosya adı. */
export function causeOf(entry: LoafEntry, view: string): { what: string; detail?: string } {
  const top = [...(entry.scripts ?? [])].sort((a, b) => b.duration - a.duration)[0]
  const file = top?.sourceURL ? top.sourceURL.split(/[\\/]/).pop() : undefined
  const source = top ? top.invoker || top.sourceFunctionName || file || 'script' : entry.entryType === 'longtask' ? 'task' : 'render'
  const where = file && top?.sourceCharPosition !== undefined && top.sourceCharPosition >= 0 ? `${file}:${top.sourceCharPosition}` : file
  return { what: `${view}: ${source}`, detail: where }
}

/** Görünür panelin kimliği (yoksa sohbet). */
function currentView(): string {
  const panel = document.querySelector<HTMLElement>('[data-kor-panel]')
  return panel?.dataset.korPanel ? `panel:${panel.dataset.korPanel}` : 'chat'
}

export function startFrameMonitor(): () => void {
  if (typeof PerformanceObserver === 'undefined') return () => {}
  const supported = PerformanceObserver.supportedEntryTypes ?? []
  const type = supported.includes('long-animation-frame') ? 'long-animation-frame' : supported.includes('longtask') ? 'longtask' : null
  if (type === null) return () => {}
  const obs = new PerformanceObserver((list) => {
    for (const e of list.getEntries() as LoafEntry[]) {
      if (e.duration <= SLOW_MS) continue
      const { what, detail } = causeOf(e, currentView())
      reportSlow({ kind: 'frame', what, ms: e.duration, detail })
    }
  })
  obs.observe({ type, buffered: false })
  return () => obs.disconnect()
}
