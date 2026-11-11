// Planlar (§28.2, ADR 0050): ajanın plan modunda `.kor/plans/` altına yazdığı planlar. Onaylamak kullanıcı
// eylemidir; onaylanınca o oturumun yazma araçları açılır, plan dışı adım yine onay ister.
// CLI: kor plan list | show <id> | approve [id]; TUI/REPL: /plan, /go.
import { CheckCircle2, ClipboardList, RefreshCw } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import type { PlanInfo } from '../../../ipc-prefs.ts'
import { api, errorMessage } from '../../api.ts'
import { planApi } from '../../prefs/api.ts'
import type { KorPanelProps } from '../registry.ts'
import '../graph/panel.css'
import '../../prefs/prefs.css'

const planEvents = (api as unknown as { plan: { onApproved(cb: (p: unknown) => void): () => void } }).plan

function PlanCard({ plan, onApproved }: { plan: PlanInfo; onApproved: () => void }) {
  const uid = useId()
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [body, setBody] = useState<string | null>(null)
  const approve = async () => {
    setBusy(true)
    setError(null)
    try {
      await planApi.approve(plan.id)
      onApproved()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  const show = async () => {
    try {
      setBody(body === null ? (await planApi.show(plan.id)).body : null)
    } catch (err) {
      setError(errorMessage(err))
    }
  }
  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <div className="kg-card-head">
        <h3 id={`${uid}-h`} className="kg-card-title">
          <span className="kg-card-name" translate="no">{plan.title}</span>
        </h3>
        <span className={`kor-tag${plan.status === 'approved' ? '' : ' muted'}`}>{plan.status === 'approved' ? 'onaylandı' : 'onay bekliyor'}</span>
      </div>
      <p className="muted kg-count" translate="no">{plan.id}</p>
      {plan.files.length > 0 && <p className="g7-note">Dosyalar: <span translate="no">{plan.files.join(', ')}</span></p>}
      <div className="kg-row">
        <button type="button" className="btn" onClick={() => void show()} data-kor-cmd="plan.show">{body === null ? 'Planı göster' : 'Gizle'}</button>
        {plan.status === 'draft' && (
          <button type="button" className="btn primary" disabled={busy} onClick={() => void approve()} data-kor-cmd="plan.approve">
            <CheckCircle2 size={14} aria-hidden="true" /> Onayla ve başlat
          </button>
        )}
      </div>
      {body !== null && <pre className="kor-prefs-pre" translate="no">{body}</pre>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

export function PlansPanel(_props: KorPanelProps) {
  const plans = useSWR('plan:list', () => planApi.list())
  useEffect(() => planEvents.onApproved(() => void plans.mutate()), [plans])
  return (
    <div className="kp" data-kor-event="kor:plan/approved">
      <div className="kg-row">
        <p className="g7-note">Plan modunda (ya da plan.lock açıkken) ajan yazamaz; planı kaydeder ve onayını bekler.</p>
        <button type="button" className="action-btn kg-push" onClick={() => void plans.mutate()} aria-label="Planları yenile" title="Yenile" data-kor-cmd="plan.list">
          <RefreshCw size={16} aria-hidden="true" />
        </button>
      </div>
      {plans.error && <p className="error-text" role="alert">{errorMessage(plans.error)}</p>}
      {!plans.data && !plans.error && <p className="muted">Yükleniyor…</p>}
      {plans.data?.length === 0 && (
        <div className="kg-empty">
          <ClipboardList size={20} aria-hidden="true" />
          <p>Bu projede plan yok. Mesaja "önce plan yap" yaz ya da izin modunu Plan yap.</p>
        </div>
      )}
      {plans.data?.map((p) => <PlanCard key={p.id} plan={p} onApproved={() => void plans.mutate()} />)}
    </div>
  )
}
