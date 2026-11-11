// `perfmon.*` köprüsü (§32.3, ADR 0062).
import { api } from '../api.ts'
import type { PerfmonApi } from '../../ipc-perfmon.ts'

export const perfmonApi = (api as unknown as { perfmon: PerfmonApi }).perfmon

/** İşlem bu karede bittikten ve ekrana çizildikten sonra çözülür (rAF + bir görev). */
export function afterPaint(): Promise<number> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => {
      const ch = new MessageChannel()
      ch.port1.onmessage = () => resolve(performance.now())
      ch.port2.postMessage(0)
    })
  })
}

/** `work` bir karenin başında çalışır; dönen süre işin ve o karenin çiziminin toplamıdır. */
export function timeFrame(work: () => void): Promise<number> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => {
      const t0 = performance.now()
      work()
      const ch = new MessageChannel()
      ch.port1.onmessage = () => resolve(performance.now() - t0)
      ch.port2.postMessage(0)
    })
  })
}
