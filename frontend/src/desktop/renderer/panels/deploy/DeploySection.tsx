// `kor deploy` (§8.6): ortamlar, değişen servisler, risk kontrolü, onaylı adımlar ve sağlık sonuçlu günlük.
import { AlertTriangle, ListChecks, Play, RefreshCw, ShieldAlert } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import type { DeployPlan, DeployReport, RiskReport } from '../../../ipc-deploy.ts'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { deployApi } from './bridge.ts'
import { defaultApproved, reduceDeploy, riskCounts, rowsOf, shortSha, type StepRow, when } from './shape.ts'

export function DeploySection() {
  const envs = useSWR('deploy:envs', () => deployApi.envs(), { revalidateOnFocus: false })
  const [env, setEnv] = useState<string>('')
  const [plan, setPlan] = useState<DeployPlan | null>(null)
  const [approved, setApproved] = useState<number[]>([])
  const [rollback, setRollback] = useState(true)
  const [risk, setRisk] = useState<RiskReport | null>(null)
  const [rows, setRows] = useState<StepRow[]>([])
  const [report, setReport] = useState<DeployReport | null>(null)
  const [running, setRunning] = useState(false)
  const [busy, setBusy] = useState<'plan' | 'risk' | null>(null)
  const [error, setError] = useState<string | null>(null)

  const names = envs.data?.envs ?? []
  const current = env || names[0] || ''

  useEffect(() => {
    const offEvent = deployApi.onEvent(({ env: e, event }) => {
      if (e === current) setRows((r) => reduceDeploy(r, event))
    })
    const offDone = deployApi.onDone((r) => {
      if (r.env !== current) return
      setReport(r)
      setRunning(false)
      void envs.mutate()
    })
    return () => {
      offEvent()
      offDone()
    }
  }, [current, envs])

  const load = async (kind: 'plan' | 'risk') => {
    setBusy(kind)
    setError(null)
    try {
      if (kind === 'plan') {
        const p = await deployApi.plan(current)
        setPlan(p)
        setApproved(defaultApproved(p))
        setRows(rowsOf(p))
        setReport(null)
      } else {
        setRisk(await deployApi.risk(current))
      }
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  const run = async () => {
    if (!plan) return
    const chosen = plan.steps.filter((s) => approved.includes(s.id))
    const destructive = chosen.filter((s) => s.destructive)
    const ok = await confirmAction({
      title: `${plan.env} ortamına dağıt`,
      message: `${chosen.length} adım çalışacak${destructive.length ? `; ${destructive.length} tanesi yıkıcı` : ''}. Onaylanmayan adımda dağıtım durur.`,
      confirmLabel: 'Dağıt',
      danger: destructive.length > 0,
    })
    if (!ok) return
    setError(null)
    setRows(rowsOf(plan))
    setReport(null)
    try {
      setRunning(true)
      await deployApi.run(plan.env, approved, plan.steps.map((s) => s.display), plan.rollback === 'ask' ? rollback : plan.rollback === 'auto')
    } catch (err) {
      setRunning(false)
      setError(errorMessage(err))
    }
  }

  if (envs.error) {
    return (
      <section className="kg-card" aria-label="Dağıtım">
        <p className="error-text" role="alert">{errorMessage(envs.error)}</p>
        <button type="button" className="btn small" onClick={() => void envs.mutate()} data-kor-cmd="deploy.envs">
          <RefreshCw size={14} aria-hidden="true" />
          Yeniden dene
        </button>
      </section>
    )
  }
  if (!envs.data) return <p className="muted pad" role="status" data-kor-cmd="deploy.envs">Ortamlar okunuyor…</p>
  const last = envs.data.state.envs[current]

  return (
    <section className="kg-card" aria-label="Dağıtım" data-kor-event="kor:deploy/event">
      <div className="kg-row">
        <label className="kg-field">
          <span>Ortam</span>
          <select value={current} onChange={(e) => { setEnv(e.target.value); setPlan(null); setRisk(null); setRows([]) }} translate="no">
            {names.map((n) => <option key={n} value={n}>{n}</option>)}
          </select>
        </label>
        <button type="button" className="btn small" disabled={busy !== null || running} onClick={() => void load('plan')} data-kor-cmd="deploy.plan">
          <ListChecks size={14} aria-hidden="true" />
          {busy === 'plan' ? 'Hesaplanıyor…' : 'Değişenleri bul'}
        </button>
        <button type="button" className="btn small" disabled={busy !== null} onClick={() => void load('risk')} data-kor-cmd="deploy.risk">
          <ShieldAlert size={14} aria-hidden="true" />
          {busy === 'risk' ? 'Kontrol ediliyor…' : 'Risk kontrolü'}
        </button>
      </div>
      <p className="muted kd-small">
        {last?.commit ? `Son dağıtım: ${shortSha(last.commit)} · ${when(last.at)}` : 'Bu ortam henüz dağıtılmadı.'}
        {last?.color ? ` · etkin renk: ${last.color}` : ''}
      </p>
      {error && <p className="error-text" role="alert">{error}</p>}
      {risk && <RiskList risk={risk} />}
      {plan && (
        <>
          <p className="kd-small">
            <span translate="no">{plan.target}</span>
            {' · '}
            {plan.services.length ? <span translate="no">{plan.services.join(', ')}</span> : 'değişen servis yok'}
            {' · '}
            {plan.changed_files === null ? 'ilk dağıtım: hepsi' : `${plan.changed_files.length} değişen dosya (${shortSha(plan.since)} sonrası)`}
          </p>
          {plan.notes.map((n) => <p key={n} className="muted kd-small" translate="no">{n}</p>)}
          {plan.steps.length === 0 ? (
            <p className="muted">Son dağıtımdan beri değişen bir şey yok.</p>
          ) : (
            <ol className="kd-steps" aria-label="Dağıtım adımları">
              {rows.map((r, i) => (
                <li key={`${r.step.id}-${i}`} className={`kd-step ${r.state}${r.step.destructive ? ' destructive' : ''}`}>
                  <label className="kd-step-head">
                    {r.step.approve && i < plan.steps.length ? (
                      <input
                        type="checkbox"
                        checked={approved.includes(r.step.id)}
                        disabled={running}
                        onChange={(e) => setApproved((a) => (e.target.checked ? [...a, r.step.id] : a.filter((x) => x !== r.step.id)))}
                        aria-label={`${r.step.id}. adımı onayla`}
                      />
                    ) : (
                      <span className="kd-read" title="Okuma adımı: onay gerekmez">•</span>
                    )}
                    <code translate="no">{r.step.display}</code>
                    {r.step.destructive && <span className="kd-tag">yıkıcı</span>}
                    <span className="kd-state">{stateText(r.state)}</span>
                  </label>
                  {r.detail && <p className="kd-detail" translate="no">{r.detail}</p>}
                  {r.output.length > 0 && <pre className="kd-out" translate="no">{r.output.join('\n')}</pre>}
                </li>
              ))}
            </ol>
          )}
          {plan.steps.length > 0 && (
            <div className="kg-row">
              {plan.rollback === 'ask' && (
                <label className="kd-check">
                  <input type="checkbox" checked={rollback} onChange={(e) => setRollback(e.target.checked)} disabled={running} />
                  Sağlık kontrolü başarısız olursa geri al
                </label>
              )}
              <button type="button" className="btn small primary" disabled={running || approved.length === 0} onClick={() => void run()} data-kor-cmd="deploy.run">
                <Play size={14} aria-hidden="true" />
                {running ? 'Dağıtılıyor…' : 'Onaylı adımları çalıştır'}
              </button>
            </div>
          )}
        </>
      )}
      {report && <ReportView report={report} />}
    </section>
  )
}

function stateText(s: StepRow['state']): string {
  return { pending: 'bekliyor', running: 'çalışıyor', ok: 'tamam', failed: 'başarısız', skipped: 'atlandı' }[s]
}

function RiskList({ risk }: { risk: RiskReport }) {
  const c = riskCounts(risk.risks)
  if (risk.risks.length === 0) return <p className="muted kd-small">Risk bulunmadı.</p>
  return (
    <div className="kd-risks" role="region" aria-label="Riskler">
      <p className="kd-small">
        <AlertTriangle size={14} aria-hidden="true" />
        {` ${c.high} yüksek · ${c.medium} orta · ${c.info} bilgi`}
      </p>
      <ul>
        {risk.risks.map((r, i) => (
          <li key={i} className={`kd-risk ${r.level}`}>
            <span translate="no">{r.title}</span>
            {r.file && <span className="muted" translate="no">{` (${r.file})`}</span>}
            <span className="muted kd-small" translate="no">{r.detail}</span>
          </li>
        ))}
      </ul>
    </div>
  )
}

function ReportView({ report }: { report: DeployReport }) {
  return (
    <div className="kd-report" role="status">
      <p className={report.ok ? '' : 'error-text'}>
        {report.ok
          ? `Dağıtıldı: ${shortSha(report.commit)}`
          : report.cancelled
            ? 'Reddedilen adımda durdu; sonrası çalışmadı.'
            : report.rolled_back
              ? 'Dağıtım başarısız; önceki sürüme geri alındı.'
              : 'Dağıtım başarısız.'}
      </p>
      {report.health.length > 0 && (
        <ul className="kd-health" aria-label="Sağlık kontrolleri">
          {report.health.map((h) => (
            <li key={h.service} className={h.ok ? 'ok' : 'failed'}>
              <span translate="no">{h.service}</span>
              {`: ${h.ok ? 'sağlıklı' : 'sağlıksız'} `}
              <span className="muted" translate="no">{h.detail}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
