// `kor audit --security` (§9): Kor kuralları + sır tarayıcı; gitleaks/semgrep kuruluysa onlar da.
import { Play } from 'lucide-react'
import { useState } from 'react'
import type { AnalysisReport } from '../../../ipc-analysis.ts'
import { errorMessage } from '../../api.ts'
import { analysisApi } from './bridge.ts'
import { ReportView } from './ReportView.tsx'
import { registerSection } from './section-registry.ts'

function SecuritySection() {
  const [report, setReport] = useState<AnalysisReport | null>(null)
  const [noExternal, setNoExternal] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async () => {
    setBusy(true)
    setError(null)
    try {
      setReport((await analysisApi.security(noExternal)).report)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <p className="ks-note muted">Gömülü sırlar, SQL/komut enjeksiyonu, zayıf kripto, kimlik doğrulamasız uç noktalar, path traversal, güvensiz serileştirme ve kapalı TLS doğrulaması. Her bulgu dosya:satır ve kuralıyla gelir; model kullanılmaz.</p>
      <div className="ks-actions">
        <button type="button" className="btn small primary" disabled={busy} onClick={() => void run()} data-kor-cmd="analysis.security">
          <Play size={14} aria-hidden="true" /> {busy ? 'Taranıyor…' : 'Taramayı çalıştır'}
        </button>
        <label className="ks-check">
          <input type="checkbox" checked={noExternal} onChange={(e) => setNoExternal(e.target.checked)} />
          gitleaks / semgrep çalıştırma
        </label>
      </div>
      {busy && !report && <p className="muted" role="status">Proje taranıyor…</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
      {report ? <ReportView report={report} /> : !busy && !error && <p className="ks-empty">Henüz tarama yapılmadı.</p>}
    </>
  )
}

registerSection({ id: 'security', label: 'Güvenlik denetimi', component: SecuritySection })
