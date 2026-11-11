// Ölü kod (§15.14 `kor dead-code [--prune]`, ADR 0047): giriş noktalarından erişilemeyen semboller, belirsizler ayrı.
// "Doğrula" silmeyi yeni bir dalda yazar, derleyici/tür denetleyici ve testlerle (sandbox'ta) doğrular, commit eder;
// gönderme ve PR açma yalnızca onaydan sonra.
import { GitPullRequest, Search, Trash2 } from 'lucide-react'
import { useId, useState } from 'react'
import type { DeadItem, DeadReport, PrunePlan } from '../../../ipc-metrics.ts'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { g3Actions } from '../../g3-bridge.ts'
import { metricsApi } from './metrics-bridge.ts'
import { CONFIDENCE_LABEL, deadWhy, pruneNote } from './metrics-shape.ts'
import { registerSection } from './section-registry.ts'
import './metrics.css'

function Items({ items }: { items: DeadItem[] }) {
  return (
    <ul className="km-list">
      {items.slice(0, 200).map((d) => (
        <li key={d.id} className="km-row">
          <span className="km-src">{CONFIDENCE_LABEL[d.confidence]}</span>
          <button type="button" className="km-loc" onClick={() => g3Actions.current?.openFile(d.file, d.line)} translate="no">{`${d.file}:${d.line}–${d.end}`}</button>
          <span translate="no">{`${d.kind} ${d.name}`}</span>
          <span className="muted">{deadWhy(d)}</span>
        </li>
      ))}
    </ul>
  )
}

function DeadCodeSection() {
  const id = useId()
  const [report, setReport] = useState<DeadReport | null>(null)
  const [plan, setPlan] = useState<PrunePlan | null>(null)
  const [medium, setMedium] = useState(false)
  const [checkCmd, setCheckCmd] = useState('')
  const [testCmd, setTestCmd] = useState('')
  const [pr, setPr] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const act = async (what: 'find' | 'prune' | 'publish') => {
    setError(null)
    if (what === 'publish') {
      if (!plan) return
      const ok = await confirmAction({
        title: 'Dal gönderilsin ve PR açılsın mı?',
        message: `${plan.branch} dalı uzak depoya gönderilecek ve ${plan.base} dalına ${plan.removed.length} sembolü silen bir pull request açılacak.`,
        confirmLabel: 'Gönder ve PR aç',
        danger: true,
      })
      if (!ok) return
    }
    setBusy(what)
    try {
      if (what === 'find') {
        setReport(await metricsApi.deadCode())
        setPlan(null)
        setPr(null)
      } else if (what === 'prune') {
        setPlan(await metricsApi.deadCodePrune(medium, checkCmd.trim() || null, testCmd.trim() || null))
        setPr(null)
      } else if (plan) {
        setPr((await metricsApi.deadCodePublish(plan.branch)).url)
      }
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="km">
      <p className="ks-note muted">Giriş noktaları: main, HTTP route, cron/tetikleyici, testler, dışa açık API, DI/reflection kayıtları; çerçeveye özel kurallar `.kor/dead-code.toml` ile genişletilir. Dinamik çağrı riski olanlar ayrı listelenir ve silinmez.</p>
      <div className="ks-actions">
        <button type="button" className="btn small primary" disabled={busy !== null} onClick={() => void act('find')} data-kor-cmd="metrics.deadCode">
          <Search size={14} aria-hidden="true" /> {busy === 'find' ? 'Aranıyor…' : 'Ölü kodu bul'}
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {report && (
        <>
          <p role="status">{`${report.symbols} sembol, ${report.entries} giriş noktası · erişilemeyen ${report.dead.length}, belirsiz ${report.ambiguous.length}`}</p>
          <p className="km-src" translate="no">{Object.entries(report.entry_rules).map(([k, v]) => `${k} ${v}`).join(' · ')}</p>
          {report.dead.length === 0 && report.ambiguous.length === 0 && <p className="muted">Erişilemeyen kod bulunamadı.</p>}
          {report.dead.length > 0 && (
            <>
              <h4 className="ks-title">Erişilemeyen</h4>
              <Items items={report.dead} />
            </>
          )}
          {report.ambiguous.length > 0 && (
            <details>
              <summary>{`Belirsiz, dinamik çağrılıyor olabilir (${report.ambiguous.length})`}</summary>
              <Items items={report.ambiguous} />
            </details>
          )}
          {report.dead.length > 0 && (
            <details className="km-dim">
              <summary>Sil ve doğrula (--prune)</summary>
              <div className="km">
                <label className="ks-label" htmlFor={`${id}-med`}>
                  <input id={`${id}-med`} type="checkbox" checked={medium} onChange={(e) => setMedium(e.target.checked)} /> Orta güvenli olanları da sil
                </label>
                <label className="km-field" htmlFor={`${id}-chk`}>
                  <span>Derleyici / tür denetleyici (boş: otomatik)</span>
                  <input id={`${id}-chk`} value={checkCmd} onChange={(e) => setCheckCmd(e.target.value)} maxLength={300} spellCheck={false} translate="no" placeholder="cargo check --all-targets" />
                </label>
                <label className="km-field" htmlFor={`${id}-tst`}>
                  <span>Test komutu (boş: yapılandırma ya da otomatik)</span>
                  <input id={`${id}-tst`} value={testCmd} onChange={(e) => setTestCmd(e.target.value)} maxLength={300} spellCheck={false} translate="no" placeholder="cargo test" />
                </label>
                <button type="button" className="btn small" disabled={busy !== null} onClick={() => void act('prune')} data-kor-cmd="metrics.deadCodePrune">
                  <Trash2 size={14} aria-hidden="true" /> {busy === 'prune' ? 'Yazılıyor ve doğrulanıyor…' : 'Yeni dalda sil ve doğrula'}
                </button>
              </div>
            </details>
          )}
        </>
      )}
      {plan && (
        <div className="km" role="status">
          <p translate="no">{plan.branch}</p>
          <p>{`Silinen ${plan.removed.length}, geri konan ${plan.restored.length}, atlanan ${plan.skipped.length}`}</p>
          {[{ label: 'Denetleyici', run: plan.check }, { label: 'Testler', run: plan.tests }].map(({ label, run }) =>
            run ? (
              <div key={label}>
                <p className={run.ok ? 'km-ok' : 'km-bad'}>
                  {`${label}: `}
                  <code translate="no">{run.command}</code>
                  {run.ok ? ' geçti' : ' başarısız'}
                </p>
                {!run.ok && <pre className="km-output" translate="no">{run.output}</pre>}
              </div>
            ) : null,
          )}
          {plan.notes.map((n) => <p key={n} className="muted">{pruneNote(n)}</p>)}
          {plan.ready && !pr && (
            <button type="button" className="btn small primary" disabled={busy !== null} onClick={() => void act('publish')} data-kor-cmd="metrics.deadCodePublish">
              <GitPullRequest size={14} aria-hidden="true" /> {busy === 'publish' ? 'Gönderiliyor…' : 'Gönder ve PR aç'}
            </button>
          )}
          {pr && <p translate="no">{`Pull request: ${pr}`}</p>}
        </div>
      )}
    </div>
  )
}

registerSection({ id: 'dead-code', label: 'Ölü kod', component: DeadCodeSection })
