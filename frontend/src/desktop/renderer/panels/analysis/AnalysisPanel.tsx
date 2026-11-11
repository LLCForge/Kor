// Analiz görünümü (§9): bölümler sections.ts'ten gelir (güvenlik, uyum; J2/J3 bölümleri tek satırla eklenir).
// Rapor doğrulama (`kor audit verify-report`) tüm bölümler için ortak.
import { ShieldCheck, ShieldX } from 'lucide-react'
import { useId, useState } from 'react'
import type { VerifyResult } from '../../../ipc-analysis.ts'
import { errorMessage } from '../../api.ts'
import type { KorPanelProps } from '../registry.ts'
import { analysisApi } from './bridge.ts'
import { analysisSections } from './section-registry.ts'
import './sections.ts'

function VerifyCard() {
  const [result, setResult] = useState<VerifyResult | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const verify = async () => {
    setBusy(true)
    setError(null)
    try {
      const r = await analysisApi.verifyReport(null)
      if (r.path) setResult(r)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const res = result?.result
  return (
    <details className="ks-more">
      <summary>Dışa aktarılmış raporu doğrula</summary>
      <p className="ks-note muted">Rapor dosyasının SHA-256 özeti denetim kaydında aranır; yanında `.minisig` varsa imza da denetlenir.</p>
      <button type="button" className="btn small" disabled={busy} onClick={() => void verify()} data-kor-cmd="analysis.verifyReport">Dosya seç ve doğrula</button>
      {result && res && (
        <p className={result.ok ? 'ks-ok' : 'ks-broken'} role="status">
          {result.ok ? <ShieldCheck size={14} aria-hidden="true" /> : <ShieldX size={14} aria-hidden="true" />}
          {result.ok
            ? `Eşleşti: ${res.linked_at ?? ''}${res.signature_ok === true ? ' · imza geçerli' : ''}`
            : !res.linked_at
              ? 'Denetim kaydında eşleşen kayıt yok: dosya değiştirilmiş ya da hiç imzalanmamış'
              : !res.chain_ok
                ? 'Denetim kaydı zinciri doğrulanmıyor'
                : 'İmza bu dosya için geçersiz'}
        </p>
      )}
      {error && <p className="error-text" role="alert">{error}</p>}
    </details>
  )
}

export function AnalysisPanel(_: KorPanelProps) {
  const id = useId()
  return (
    <div className="ks ka">
      {analysisSections().map((s) => (
        <section key={s.id} className="kg-card" aria-labelledby={`${id}-${s.id}`}>
          <h3 id={`${id}-${s.id}`} className="kg-card-title ks-title">{s.label}</h3>
          <s.component />
        </section>
      ))}
      <section className="kg-card" aria-label="Rapor doğrulama">
        <VerifyCard />
      </section>
    </div>
  )
}
