// §32 (ADR 0062) açılış: ilk çizimden sonra `perfmon.ready` (açılış süresi), düşük güç modu, perf.log açıksa
// yavaş kare/IPC kaydı, gizli Ctrl+Shift+P bindirmesi ve ölçüm modunda `kor bench --ui` koşusu.
import { createRoot } from 'react-dom/client'
import { perfmonApi, afterPaint } from './api.ts'
import { PerfOverlay } from './Overlay.tsx'
import { startFrameMonitor } from './frames.ts'
import { setSlowSink } from './stats.ts'
import { throttled } from './util.ts'

function applyLowPower(on: boolean): void {
  if (on) document.documentElement.dataset.lowPower = '1'
  else delete document.documentElement.dataset.lowPower
}

export async function bootPerf(): Promise<void> {
  // Pencere ilk kez çizildi ve etkileşime hazır: açılış süresi burada biter (§32.1).
  await afterPaint()
  const ready = perfmonApi.ready().then(
    () => true,
    () => false,
  )
  const host = document.createElement('div')
  host.id = 'kor-perf-overlay'
  document.body.appendChild(host)
  createRoot(host).render(<PerfOverlay />)
  let cfg: Awaited<ReturnType<typeof perfmonApi.config>>
  try {
    cfg = await perfmonApi.config()
  } catch {
    return
  }
  applyLowPower(cfg.lowPower)
  // Pil durumu değişebilir: pencereye dönüşte yeniden bakılır (yoklama yok).
  window.addEventListener('focus', () => {
    perfmonApi.config().then((c) => applyLowPower(c.lowPower), () => {})
  })
  if (cfg.log) {
    setSlowSink(throttled((e) => void perfmonApi.log(e).catch(() => {})))
    startFrameMonitor()
  }
  if ((await ready) && cfg.bench === 'full') {
    const { runBench } = await import('./bench.tsx')
    await runBench()
  }
}
