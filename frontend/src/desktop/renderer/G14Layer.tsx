// [g14] Gözlem katmanı: pano penceresi, uyarı bildirimleri ve mesaj kutusundaki sağlık rozeti + son yanıt değerlendirmesi.
// Composer/Sidebar'a içe aktarma eklenmez: rozet Composer'ın [g14] yuvasına portal ile, pano pencere olayıyla açılır.
import { ThumbsDown, ThumbsUp, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import useSWR from 'swr'
import type { G14Alert, ProviderState } from '../ipc-g14.ts'
import { api, errorMessage } from './api.ts'
import { onOpenObservability, type G14Tab } from './g14-bus.ts'
import { ObservabilityPanel } from './ObservabilityPanel.tsx'

const ALERT_MS = 8_000

export const PROVIDER_LABEL: Record<ProviderState, string> = {
  ok: 'Sağlayıcı sağlıklı',
  degraded: 'Sağlayıcı yavaş veya ara sıra hata veriyor',
  down: 'Sağlayıcı hata veriyor',
  unknown: 'Henüz model çağrısı yok',
}

interface G14LayerProps {
  /** Etkin sohbet (yeni sohbet taslağında null) */
  sessionId: string | null
  onModelChange: (model: string) => void
}

export function G14Layer({ sessionId, onModelChange }: G14LayerProps) {
  const [open, setOpen] = useState(false)
  const [tab, setTab] = useState<G14Tab>('overview')
  const [slot, setSlot] = useState<HTMLElement | null>(null)
  const [alerts, setAlerts] = useState<Array<G14Alert & { id: number }>>([])

  useEffect(() => {
    const show = (next: G14Tab) => {
      setTab(next)
      setOpen(true)
    }
    const onWindowOpen = () => show('overview')
    const onSlot = (e: Event) => {
      if (e instanceof CustomEvent && e.detail instanceof HTMLElement) setSlot(e.detail)
    }
    window.addEventListener('g14:open', onWindowOpen)
    window.addEventListener('g14:slot', onSlot)
    // Yuva bu katmandan önce bağlanmış olabilir (olay kaçırıldı).
    setSlot(document.querySelector<HTMLElement>('.g14-slot'))
    const offBus = onOpenObservability(show)
    let seq = 0
    const offAlert = api.g14.onAlert((alert) => {
      const id = ++seq
      setAlerts((prev) => [...prev.slice(-2), { ...alert, id }])
      setTimeout(() => setAlerts((prev) => prev.filter((a) => a.id !== id)), ALERT_MS)
    })
    return () => {
      window.removeEventListener('g14:open', onWindowOpen)
      window.removeEventListener('g14:slot', onSlot)
      offBus()
      offAlert()
    }
  }, [])

  const openTab = (next: G14Tab) => {
    setTab(next)
    setOpen(true)
  }

  return (
    <>
      {slot && createPortal(<ComposerWidget sessionId={sessionId} onOpen={() => openTab('overview')} />, slot)}
      <ObservabilityPanel open={open} tab={tab} onTab={setTab} onClose={() => setOpen(false)} sessionId={sessionId} onModelChange={onModelChange} />
      <div className="g14-toasts" role="status" aria-live="polite">
        {alerts.map((a) => (
          <div key={a.id} className={`g14-toast ${a.kind}`}>
            <span className="g14-toast-kind">{a.kind === 'slowTool' ? 'Yavaş araç' : a.kind === 'turnBudget' ? 'Tur bütçesi' : 'Bellek bütçesi'}</span>
            <span className="g14-toast-text">{a.message}</span>
            <button type="button" className="btn small ghost" onClick={() => openTab(a.kind === 'slowTool' ? 'overview' : a.kind === 'memoryBudget' ? 'processes' : 'traces')}>
              İncele
            </button>
            <button type="button" className="action-btn" aria-label="Uyarıyı kapat" onClick={() => setAlerts((prev) => prev.filter((x) => x.id !== a.id))}>
              <X size={14} aria-hidden="true" />
            </button>
          </div>
        ))}
      </div>
    </>
  )
}

function ComposerWidget({ sessionId, onOpen }: { sessionId: string | null; onOpen: () => void }) {
  const [tick, setTick] = useState(0)
  const [error, setError] = useState<string | null>(null)
  useEffect(
    () =>
      api.onEvent((e) => {
        if (e.sessionId === sessionId && (e.type === 'done' || e.type === 'error')) setTick((t) => t + 1)
      }),
    [sessionId],
  )
  const { data, mutate } = useSWR(['g14:status', sessionId, tick], () => api.g14.status(sessionId), { refreshInterval: 30_000, revalidateOnFocus: false })

  const state: ProviderState = data?.online === false ? 'down' : (data?.provider ?? 'unknown')
  const healthLabel = data?.online === false ? 'Ağ bağlantısı yok' : PROVIDER_LABEL[state]
  const last = data?.lastTurn ?? null

  const rate = async (rating: 1 | -1) => {
    if (!last) return
    setError(null)
    try {
      await api.g14.rate(last.traceId, last.rating === rating ? 0 : rating)
      await mutate()
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  return (
    <span className="g14-widget">
      <button type="button" className="action-btn g14-health" onClick={onOpen} aria-label={`Gözlem panosu. ${healthLabel}`} title={`${healthLabel}. Gözlem panosunu aç`}>
        <span className={`g14-dot ${state}`} aria-hidden="true" />
      </button>
      {last && (
        <>
          <button
            type="button"
            className={`action-btn g14-rate${last.rating === 1 ? ' active' : ''}`}
            onClick={() => void rate(1)}
            aria-pressed={last.rating === 1}
            aria-label={`Son yanıtı beğen (${last.model})`}
            title={error ?? `Son yanıtı beğen: model önerisini etkiler (${last.model})`}
          >
            <ThumbsUp size={14} aria-hidden="true" />
          </button>
          <button
            type="button"
            className={`action-btn g14-rate${last.rating === -1 ? ' active' : ''}`}
            onClick={() => void rate(-1)}
            aria-pressed={last.rating === -1}
            aria-label={`Son yanıtı beğenme (${last.model})`}
            title={error ?? `Son yanıtı beğenme: model önerisini etkiler (${last.model})`}
          >
            <ThumbsDown size={14} aria-hidden="true" />
          </button>
        </>
      )}
    </span>
  )
}
