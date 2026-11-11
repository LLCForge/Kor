// Kalite (§9 `kor quality`, ADR 0047): boyut başına A–F not, her sayının kaynağı, formül, iyileştirme listesi ve
// commit başına anlık görüntülerden eğilim. Kapsam raporu yoksa "veri yok" der; Kor kapsam aracı çalıştırmaz.
import { Gauge, History } from 'lucide-react'
import { useState } from 'react'
import type { QualityReport, QualitySnapshot } from '../../../ipc-metrics.ts'
import { errorMessage } from '../../api.ts'
import { g3Actions } from '../../g3-bridge.ts'
import { metricsApi } from './metrics-bridge.ts'
import { day, DIM_LABEL, formulaFor, METRIC_LABEL, metricValue, NOTE, num, signed } from './metrics-shape.ts'
import { registerSection } from './section-registry.ts'
import './metrics.css'

function QualitySection() {
  const [report, setReport] = useState<QualityReport | null>(null)
  const [history, setHistory] = useState<QualitySnapshot[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async (what: 'quality' | 'history') => {
    setBusy(true)
    setError(null)
    try {
      if (what === 'quality') setReport(await metricsApi.quality())
      else setHistory((await metricsApi.qualityHistory()).snapshots)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="km">
      <p className="ks-note muted">tree-sitter ölçümleri, var olan kapsam raporu, Kor güvenlik kuralları ve `kor perf` kayıtlarından. Her ölçüm commit’e göre `.kor/quality/` altında saklanır.</p>
      <div className="ks-actions">
        <button type="button" className="btn small primary" disabled={busy} onClick={() => void run('quality')} data-kor-cmd="metrics.quality">
          <Gauge size={14} aria-hidden="true" /> {busy ? 'Ölçülüyor…' : 'Kaliteyi ölç'}
        </button>
        <button type="button" className="btn small" disabled={busy} onClick={() => void run('history')} data-kor-cmd="metrics.qualityHistory">
          <History size={14} aria-hidden="true" /> Geçmiş
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {history && (
        history.length === 0 ? (
          <p className="muted">Henüz anlık görüntü yok.</p>
        ) : (
          <table className="km-table">
            <thead>
              <tr><th scope="col">Tarih</th><th scope="col">Commit</th><th scope="col">Genel</th><th scope="col">Borç</th></tr>
            </thead>
            <tbody>
              {history.map((s) => (
                <tr key={s.commit}>
                  <td>{day(s.taken_at)}</td>
                  <td translate="no">{s.commit.slice(0, 10)}</td>
                  <td>{s.overall === null ? '—' : num(s.overall)}</td>
                  <td>{`≈ ${num(s.debt_hours)} sa`}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )
      )}
      {report && <Report r={report} />}
    </div>
  )
}

function Report({ r }: { r: QualityReport }) {
  return (
    <>
      <div className="km-head" role="status">
        <span className={`km-grade km-grade-${r.grade ?? 'none'}`}>{r.grade ?? '—'}</span>
        <span>{r.overall === null ? 'Not yok: hiçbir boyutta veri yok' : `${num(r.overall)}/100 · ${r.files} dosya`}</span>
        {r.commit && <span className="muted" translate="no">{r.commit.slice(0, 10)}</span>}
        {r.dirty && <span className="muted">commit edilmemiş değişiklikler dahil</span>}
      </div>
      <div className="km-dims">
        {r.dimensions.map((d) => (
          <details key={d.key} className="km-dim">
            <summary>
              <span className={`km-grade km-grade-${d.grade ?? 'none'}`}>{d.grade ?? '—'}</span>
              <span>{DIM_LABEL[d.key]}</span>
              <span className="muted">{d.score === null ? 'veri yok' : `${num(d.score)}/100`}</span>
            </summary>
            {d.score !== null && <p className="km-formula">{`Formül: ${formulaFor(d)}`}</p>}
            {d.note && <p className="muted">{NOTE[d.note] ?? d.note}</p>}
            {d.metrics.length > 0 && (
              <table className="km-table">
                <tbody>
                  {d.metrics.map((m) => (
                    <tr key={m.key}>
                      <td>{METRIC_LABEL[m.key] ?? m.key}</td>
                      <td>{metricValue(m.unit, m.value)}</td>
                      <td className="km-src" translate="no">{m.source}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </details>
        ))}
      </div>
      <p>{`Teknik borç: ≈ ${num(r.debt_hours)} sa`}</p>
      {r.trend && (
        <p className="muted">
          {`${r.trend.since.slice(0, 10)} (${day(r.trend.since_at)}) sonrasından: genel ${r.trend.overall === null ? '—' : signed(r.trend.overall)} puan, borç ${signed(r.trend.debt_hours)} sa`}
          {r.trend.dimensions.map(([k, v]) => ` · ${DIM_LABEL[k as keyof typeof DIM_LABEL] ?? k} ${signed(v)}`).join('')}
        </p>
      )}
      {r.findings.length > 0 && (
        <>
          <h4 className="ks-title">{`Önce bunları iyileştirin (${r.findings_total} bulgudan ${r.findings.length})`}</h4>
          <ul className="km-list">
            {r.findings.map((f) => (
              <li key={`${f.rule}:${f.file}:${f.line}`} className="km-row">
                <button type="button" className="km-loc" onClick={() => g3Actions.current?.openFile(f.file, f.line)} translate="no">{`${f.file}:${f.line}`}</button>
                <span>{f.message}</span>
                <span className="km-src" translate="no">{f.rule}</span>
              </li>
            ))}
          </ul>
        </>
      )}
    </>
  )
}

registerSection({ id: 'quality', label: 'Kalite', component: QualitySection })
