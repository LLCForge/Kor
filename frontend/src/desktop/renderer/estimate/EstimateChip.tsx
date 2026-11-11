// §27.16 composer altındaki tahmin (ADR 0063): yazarken "≈ 4 dk · ≈ $0.03 · ~12 dosya" (yöntemi
// ipucunda); görev çalışırken geçen süre / tahmin, sapma büyürse uyarı rengi. Bütçe sınırı
// aşılacaksa uyarı gösterilir; ajan da ilk model çağrısından önce durup sorar.
import { Hourglass } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import { api } from '../api.ts'
import { estimateApi } from '../ek14-api.ts'
import { estimable, progressLine } from './estimate.ts'
import './estimate.css'

interface Props {
  draft: string
  sessionId: string | null
  model: string
  running: boolean
}

interface RunEstimate { line: string; secs: number }

export function EstimateChip({ draft, sessionId, model, running }: Props) {
  // Yazmayı yavaşlatmasın: taslak 600 ms durunca sorulur.
  const [settled, setSettled] = useState(draft)
  useEffect(() => {
    const t = window.setTimeout(() => setSettled(draft), 600)
    return () => window.clearTimeout(t)
  }, [draft])
  const key = !running && estimable(settled) ? ['estimate', settled, sessionId ?? '', model] : null
  const { data } = useSWR(key, () => estimateApi.task(settled, sessionId, model || null), { revalidateOnFocus: false, shouldRetryOnError: false })

  // Çalışan görevin tahmini ajandan `estimate` olayıyla gelir.
  const [run, setRun] = useState<{ est: RunEstimate; started: number } | null>(null)
  const [now, setNow] = useState(() => Date.now())
  useEffect(
    () =>
      api.onEvent((e) => {
        const ev = e as { type: string; sessionId?: string; line?: string; secs?: number }
        if (ev.type === 'estimate' && (!sessionId || ev.sessionId === sessionId) && ev.line && ev.secs != null) {
          setRun({ est: { line: ev.line, secs: ev.secs }, started: Date.now() })
        }
      }),
    [sessionId],
  )
  useEffect(() => {
    if (!running) {
      setRun(null)
      return
    }
    // Gerçek geçen süre: saniyede bir yeniden çizilir (sahte sayaç değil, Date.now()).
    const t = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(t)
  }, [running])

  if (running && run) {
    const p = progressLine((now - run.started) / 1000, run.est.secs)
    return (
      <span className={`k14-estimate${p.drift ? ' drift' : ''}`} data-kor-event="kor:event" title={run.est.line}>
        <Hourglass size={12} aria-hidden="true" /> {p.text}
      </span>
    )
  }
  if (!data) return null
  return (
    <span className={`k14-estimate${data.warning ? ' drift' : ''}`} data-kor-cmd="estimate.task" title={data.warning ? `${data.method}\n${data.warning}` : data.method}>
      <Hourglass size={12} aria-hidden="true" /> {data.line}
    </span>
  )
}
