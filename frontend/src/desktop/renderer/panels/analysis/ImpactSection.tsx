// Etki (§9 `kor impact`, ADR 0047): değiştirmeden önce dosya/sembol/diff'in etkilediği dosya, uç nokta, migration,
// tablo, paket ve test sayıları, formülüyle tahmini süre (≈).
import { Radar } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import type { ImpactMode, ImpactReport } from '../../../ipc-metrics.ts'
import { errorMessage } from '../../api.ts'
import { g3Actions } from '../../g3-bridge.ts'
import { metricsApi } from './metrics-bridge.ts'
import { IMPACT_FORMULA, num } from './metrics-shape.ts'
import { registerSection } from './section-registry.ts'
import './metrics.css'

function Names({ title, list }: { title: string; list: string[] }) {
  return (
    <p>
      {`${title}: ${list.length}`}
      {list.length > 0 && <span className="km-src" translate="no">{` ${list.slice(0, 12).join(', ')}${list.length > 12 ? ', …' : ''}`}</span>}
    </p>
  )
}

function ImpactSection() {
  const id = useId()
  const [target, setTarget] = useState('')
  const [mode, setMode] = useState<ImpactMode>('name')
  const [report, setReport] = useState<ImpactReport | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      setReport(await metricsApi.impact(mode === 'name' ? target.trim() : null, mode, null))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="km">
      <form className="km-row" onSubmit={(e) => void submit(e)}>
        <label className="km-field" htmlFor={`${id}-m`}>
          <span>Ne değişecek</span>
          <select id={`${id}-m`} value={mode} onChange={(e) => setMode(e.target.value as ImpactMode)}>
            <option value="name">Dosya ya da sembol</option>
            <option value="working">Commit edilmemiş değişiklikler</option>
            <option value="staged">Sahnelenen değişiklikler</option>
          </select>
        </label>
        {mode === 'name' && (
          <label className="km-field" htmlFor={`${id}-t`}>
            <span>Dosya ya da sembol</span>
            <input id={`${id}-t`} value={target} onChange={(e) => setTarget(e.target.value)} maxLength={500} spellCheck={false} translate="no" placeholder="src/db.rs" />
          </label>
        )}
        <button type="submit" className="btn small primary" disabled={busy || (mode === 'name' && !target.trim())} data-kor-cmd="metrics.impact">
          <Radar size={14} aria-hidden="true" /> {busy ? 'Hesaplanıyor…' : 'Etkiyi hesapla'}
        </button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
      {report && (
        <div role="status">
          {report.not_found.length > 0 && <p className="muted" translate="no">{`Grafta yok: ${report.not_found.join(', ')}`}</p>}
          <Names title="Dosyalar" list={report.files} />
          <p>{`Semboller: ${report.symbols}`}</p>
          <p>{`HTTP uç noktaları: ${report.endpoints.length}`}</p>
          {report.endpoints.length > 0 && (
            <ul className="km-list">
              {report.endpoints.slice(0, 15).map((ep) => (
                <li key={`${ep.file}:${ep.line}:${ep.name}`} className="km-row">
                  <button type="button" className="km-loc" onClick={() => g3Actions.current?.openFile(ep.file, ep.line)} translate="no">{`${ep.file}:${ep.line}`}</button>
                  <span translate="no">{`${ep.name} (${ep.rule})`}</span>
                </li>
              ))}
            </ul>
          )}
          <Names title="Migration’lar" list={report.migrations} />
          <Names title="Tablolar" list={report.tables} />
          <Names title="Paketler" list={report.packages} />
          <Names title="Testler" list={report.tests} />
          {report.truncated && <p className="muted">Liste 500 düğümde kesildi.</p>}
          <p>{`Tahmini süre: ≈ ${num(report.hours)} sa`}</p>
          <p className="km-formula">{`Formül: ${IMPACT_FORMULA}`}</p>
        </div>
      )}
    </div>
  )
}

registerSection({ id: 'impact', label: 'Etki', component: ImpactSection })
