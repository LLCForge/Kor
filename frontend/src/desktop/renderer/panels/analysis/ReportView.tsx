// Bir analiz raporunun ortak görünümü: önem süzgeçleri, arama, dosya:satır'da açılan bulgu listesi,
// dışa aktarma (md/json/sarif, isteğe bağlı imza). Diğer bölümler (lisans, kalite) de bunu kullanabilir.
import { Download, FileCode2 } from 'lucide-react'
import { useId, useMemo, useState } from 'react'
import type { AnalysisReport, ExportFormat, Severity } from '../../../ipc-analysis.ts'
import { errorMessage } from '../../api.ts'
import { g3Actions } from '../../g3-bridge.ts'
import { analysisApi } from './bridge.ts'
import { counts, filterFindings, location, SEVERITIES, SEVERITY_LABELS } from './shape.ts'

const FORMATS: readonly { id: ExportFormat; label: string }[] = [
  { id: 'md', label: 'Markdown' },
  { id: 'json', label: 'JSON' },
  { id: 'sarif', label: 'SARIF' },
]

/** Listede bir kerede çizilen en çok bulgu; fazlası için süzgeç kullanılır. */
const MAX_SHOWN = 300

export function ReportView({ report }: { report: AnalysisReport }) {
  const id = useId()
  const [shown, setShown] = useState<ReadonlySet<Severity>>(() => new Set(SEVERITIES))
  const [query, setQuery] = useState('')
  const [sign, setSign] = useState(false)
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  const c = useMemo(() => counts(report.findings), [report])
  const list = useMemo(() => filterFindings(report.findings, shown, query), [report, shown, query])

  const toggle = (s: Severity) => {
    const next = new Set(shown)
    if (next.has(s)) next.delete(s)
    else next.add(s)
    setShown(next)
  }

  const exportAs = async (format: ExportFormat) => {
    setBusy(true)
    setError(null)
    setStatus(null)
    try {
      const r = await analysisApi.export(report.tool, format, sign)
      if (!r.path) return
      const parts = [`Rapor yazıldı: ${r.path}`]
      if (r.sha256) parts.push(`denetim kaydına bağlandı (sha256 ${r.sha256.slice(0, 12)}…)`)
      if (r.signature) parts.push(`imza: ${r.signature}`)
      setStatus(parts.join(' · '))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="ka-report">
      <fieldset className="ka-sev">
        <legend className="sr-only">Önem süzgeci</legend>
        {SEVERITIES.map((s) => (
          <label key={s} className={`ka-sev-chip ka-${s}${shown.has(s) ? ' on' : ''}`}>
            <input type="checkbox" checked={shown.has(s)} onChange={() => toggle(s)} />
            {SEVERITY_LABELS[s]} <span className="ka-count">{c[s]}</span>
          </label>
        ))}
      </fieldset>
      <label className="sr-only" htmlFor={`${id}-q`}>Bulgularda ara</label>
      <input id={`${id}-q`} className="text-input" type="search" placeholder="Kural, dosya, CWE…" value={query} onChange={(e) => setQuery(e.target.value)} />

      {report.findings.length === 0 ? (
        <p className="ks-empty">Bulgu yok.</p>
      ) : list.length === 0 ? (
        <p className="ks-empty">Süzgece uyan bulgu yok.</p>
      ) : (
        <ul className="ka-list" aria-label="Bulgular">
          {list.slice(0, MAX_SHOWN).map((f) => (
            <li key={`${f.file}:${f.line}:${f.rule}`} className="ka-item">
              <div className="ka-item-head">
                <span className={`ka-badge ka-${f.severity}`}>{SEVERITY_LABELS[f.severity]}</span>
                <button
                  type="button"
                  className="ka-loc"
                  onClick={() => g3Actions.current?.openFile(f.file, f.line)}
                  title="Dosyayı bu satırda aç"
                  translate="no"
                >
                  <FileCode2 size={14} aria-hidden="true" />
                  {location(f)}
                </button>
              </div>
              <p className="ka-msg" translate="no">{f.message}</p>
              <p className="ka-meta muted">
                <code translate="no">{f.rule}</code> · <span translate="no">{f.source}</span>
                {f.cwe && <> · <span translate="no">{f.cwe}</span></>}
              </p>
              {f.fix && <p className="ka-fix" translate="no">{f.fix}</p>}
            </li>
          ))}
        </ul>
      )}
      {list.length > MAX_SHOWN && <p className="ks-note muted">{`İlk ${MAX_SHOWN} bulgu gösteriliyor (${list.length}); süzgeçle daraltın.`}</p>}

      {report.notes.length > 0 && (
        <details className="ks-more">
          <summary>Tarayıcı notları ({report.notes.length})</summary>
          <ul className="ka-notes">
            {report.notes.map((n) => <li key={n} translate="no">{n}</li>)}
          </ul>
        </details>
      )}

      <div className="ks-actions">
        {FORMATS.map((fm) => (
          <button key={fm.id} type="button" className="btn small" disabled={busy} onClick={() => void exportAs(fm.id)} data-kor-cmd="analysis.export">
            <Download size={14} aria-hidden="true" /> {fm.label}
          </button>
        ))}
        <label className="ks-check">
          <input type="checkbox" checked={sign} onChange={(e) => setSign(e.target.checked)} />
          Denetim kaydına bağla ve imzala
        </label>
      </div>
      {status && <p className="ks-note" role="status">{status}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}
