// [perf] Akan metin ve düşünme parçaları kare başına tek dispatch'te birleşir; diğer olaylar önce bekleyenleri
// boşaltır, böylece olay sırası korunur.
import type { AgentEventBody } from '../ipc.ts'

type Deliver = (sessionId: string, event: AgentEventBody, at: number) => void
/** fn'i bir kez çalıştırmak üzere planlar; dönen fonksiyon planı iptal eder. */
type Schedule = (fn: () => void) => () => void

/** Pencere gizliyken requestAnimationFrame çalışmaz; zamanlayıcı akışın takılmasını önler. */
const HIDDEN_FLUSH_MS = 100

const nextFrame: Schedule = (fn) => {
  const frame = requestAnimationFrame(fn)
  const timer = setTimeout(fn, HIDDEN_FLUSH_MS)
  return () => {
    cancelAnimationFrame(frame)
    clearTimeout(timer)
  }
}

interface Pending {
  sessionId: string
  event: Extract<AgentEventBody, { type: 'text' | 'g3:reasoning' }>
  at: number
}

export function createStreamCoalescer(deliver: Deliver, schedule: Schedule = nextFrame) {
  let pending: Pending[] = []
  let cancel: (() => void) | null = null

  const flush = () => {
    cancel?.()
    cancel = null
    const batch = pending
    pending = []
    for (const p of batch) deliver(p.sessionId, p.event, p.at)
  }

  const push = (sessionId: string, event: AgentEventBody, at: number) => {
    if (event.type !== 'text' && event.type !== 'g3:reasoning') {
      flush()
      deliver(sessionId, event, at)
      return
    }
    // Yalnızca aynı sohbetin bir önceki parçasıyla birleşir: metin ve düşünme birbirine karışmaz.
    const last = pending.findLast((p) => p.sessionId === sessionId)
    if (last?.event.type === event.type) {
      last.event = { ...event, delta: last.event.delta + event.delta }
      last.at = at
    } else {
      pending.push({ sessionId, event, at })
    }
    cancel ??= schedule(flush)
  }

  return { push, flush }
}
