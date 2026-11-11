// `kor audit --compliance=kvkk|gdpr` (§9): kişisel veri işleme noktaları, loglarda PII, saklama süresi.
import { Play } from 'lucide-react'
import { useId, useState } from 'react'
import type { AnalysisReport } from '../../../ipc-analysis.ts'
import { errorMessage } from '../../api.ts'
import { analysisApi } from './bridge.ts'
import { ReportView } from './ReportView.tsx'
import { registerSection } from './section-registry.ts'

function ComplianceSection() {
  const id = useId()
  const [regime, setRegime] = useState<'kvkk' | 'gdpr'>('kvkk')
  const [report, setReport] = useState<AnalysisReport | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async () => {
    setBusy(true)
    setError(null)
    try {
      setReport((await analysisApi.compliance(regime)).report)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <p className="ks-note muted">Kişisel veri sütunları (şema ve ORM modelleri), bu tabloları okuyan/yazan fonksiyonlar, log çağrılarına giden kişisel veri ve silme/saklama yolu olmayan tablolar. Madde atfı yalnızca eşleşme açıksa eklenir.</p>
      <div className="ks-actions">
        <label htmlFor={`${id}-r`} className="ks-label">Düzenleme</label>
        <select id={`${id}-r`} value={regime} onChange={(e) => setRegime(e.target.value === 'gdpr' ? 'gdpr' : 'kvkk')}>
          <option value="kvkk">KVKK</option>
          <option value="gdpr">GDPR</option>
        </select>
        <button type="button" className="btn small primary" disabled={busy} onClick={() => void run()} data-kor-cmd="analysis.compliance">
          <Play size={14} aria-hidden="true" /> {busy ? 'Taranıyor…' : 'Uyum taraması'}
        </button>
      </div>
      {busy && !report && <p className="muted" role="status">Proje taranıyor…</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
      {report ? <ReportView report={report} /> : !busy && !error && <p className="ks-empty">Henüz tarama yapılmadı.</p>}
    </>
  )
}

registerSection({ id: 'compliance', label: 'KVKK / GDPR uyumu', component: ComplianceSection })
