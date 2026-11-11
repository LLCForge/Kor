// §27.8 her-zaman-üstte durum penceresi (≈ 240×72, çerçevesiz, sürüklenebilir): çekirdeğin `KorStatus` fiili,
// süre, maliyet, ilerleme çizgisi, ETA geri sayımı; onay bekliyorsa Onayla / Reddet / Aç; bitince yeşil tik.
import { Check, CircleCheck, ExternalLink, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import type { RunningSession } from '../../ipc-shellui.ts'
import { errorMessage } from '../api.ts'
import { shellui, useShellState } from './bridge.ts'
import { approvalFor, countdown, elapsed, percent, primarySession, usd } from './state.ts'

export function PipView() {
  const { data, error } = useShellState()
  const [problem, setProblem] = useState<string | null>(null)
  const [finished, setFinished] = useState<RunningSession | null>(null)
  const last = useRef<RunningSession | null>(null)
  const sessions = data?.overview.sessions ?? []
  const current = primarySession(sessions)

  // Bitince kısa özet: son görülen görev listeden çıkınca yeşil tik.
  useEffect(() => {
    if (current) {
      last.current = current
      setFinished(null)
    } else if (last.current) {
      setFinished(last.current)
      last.current = null
    }
  }, [current])

  const opacity = data?.pip.opacity ?? 0.95
  const style = { ['--ksh-pip-alpha' as string]: String(opacity) }
  const approval = current ? approvalFor(data?.overview.approvals ?? [], current.id) : null
  const answer = async (approve: boolean) => {
    if (!approval) return
    setProblem(null)
    try {
      if (approve) await shellui.approve(approval.id)
      else await shellui.deny(approval.id)
    } catch (err) {
      setProblem(errorMessage(err))
    }
  }

  let body
  if (error) {
    body = <span className="ksh-pip-line error-text">Durum alınamadı</span>
  } else if (current) {
    const pct = percent(current)
    const left = countdown(current.eta, Date.now())
    body = (
      <>
        <div className="ksh-pip-line" data-tauri-drag-region>
          {current.waiting ? <span className="ksh-pip-dot warn" aria-hidden="true" /> : <span className="ksh-pip-spinner" aria-hidden="true" />}
          <strong className="ksh-pip-verb" data-tauri-drag-region>
            {current.waiting ? 'Onay bekliyor' : current.verb}
          </strong>
          <span className="ksh-muted" data-tauri-drag-region>
            {elapsed(current.since, Date.now())}
          </span>
          <span className="ksh-muted" data-tauri-drag-region>
            {usd(current.costUsd, current.costEstimated)}
          </span>
          {left && (
            <span className="ksh-muted" title={current.eta ? 'Benzer turların sürelerinden tahmin' : undefined}>
              {left}
            </span>
          )}
        </div>
        {approval ? (
          <div className="ksh-pip-actions">
            <button type="button" className="btn small primary" onClick={() => void answer(true)} data-kor-cmd="shellui.approve">
              <Check size={12} aria-hidden="true" /> Onayla
            </button>
            <button type="button" className="btn small" onClick={() => void answer(false)} data-kor-cmd="shellui.deny">
              <X size={12} aria-hidden="true" /> Reddet
            </button>
            <button type="button" className="btn small" onClick={() => void shellui.openMain(current.id)} data-kor-cmd="shellui.openMain">
              <ExternalLink size={12} aria-hidden="true" /> Aç
            </button>
          </div>
        ) : (
          <div className={`ksh-pip-bar${pct === null ? ' indeterminate' : ''}`} role="progressbar" aria-label="İlerleme" aria-valuemin={0} aria-valuemax={100} {...(pct !== null ? { 'aria-valuenow': pct } : {})}>
            <span style={pct === null ? undefined : { width: `${pct}%` }} />
          </div>
        )}
        <span className="ksh-pip-title ksh-muted" translate="no" data-tauri-drag-region>
          {current.title}
          {sessions.length > 1 ? ` · +${sessions.length - 1}` : ''}
        </span>
      </>
    )
  } else if (finished) {
    body = (
      <div className="ksh-pip-line" data-tauri-drag-region>
        <CircleCheck size={16} className="ksh-ok" aria-hidden="true" />
        <strong data-tauri-drag-region>Bitti</strong>
        <span className="ksh-muted" translate="no" data-tauri-drag-region>
          {finished.title}
        </span>
      </div>
    )
  } else {
    body = (
      <div className="ksh-pip-line" data-tauri-drag-region>
        <span className="ksh-muted" data-tauri-drag-region>
          Çalışan görev yok
        </span>
      </div>
    )
  }
  return (
    <main className="ksh-pip" style={style} data-tauri-drag-region data-kor-cmd="shellui.state" data-kor-event="kor:shellui/changed">
      {body}
      <button type="button" className="action-btn ksh-pip-close" aria-label="Durum penceresini gizle" title="Durum penceresini gizle" onClick={() => void shellui.pip(false)} data-kor-cmd="shellui.pip">
        <X size={12} aria-hidden="true" />
      </button>
      {problem && (
        <span className="error-text ksh-pip-error" role="alert">
          {problem}
        </span>
      )}
    </main>
  )
}
