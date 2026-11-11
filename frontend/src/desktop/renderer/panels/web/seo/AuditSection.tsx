// Teknik SEO denetimi (§29.5, §29.16): robots.txt'e uyan, hız sınırlı tarama; bulgular URL, satır ve kural ile.
// Core Web Vitals Kor'un başsız tarayıcısında ölçülür (laboratuvar verisi). CLI: kor seo audit <url|sitemap>.
import { CalendarClock, CheckCheck, ScanSearch } from 'lucide-react'
import { useEffect, useId, useState, type FormEvent } from 'react'
import type { SeoAuditResult, SeoDepth, SeoVerified } from '../../../../ipc-seo.ts'
import { errorMessage } from '../../../api.ts'
import { seoApi } from './api.ts'
import { Findings } from './Findings.tsx'

export function AuditSection() {
  const uid = useId()
  const [target, setTarget] = useState('')
  const [depth, setDepth] = useState<SeoDepth>('quick')
  const [vitals, setVitals] = useState(true)
  const [busy, setBusy] = useState(false)
  const [progress, setProgress] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<SeoAuditResult | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [verified, setVerified] = useState<SeoVerified | null>(null)

  useEffect(() => seoApi.onProgress((p) => {
    if (p.kind === 'audit' && p.detail) setProgress(p.detail)
  }), [])

  const submit = (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    setProgress('')
    seoApi
      .audit(target.trim(), depth, null, vitals)
      .then(setResult)
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setBusy(false))
  }

  const verify = () => {
    setBusy(true)
    setError(null)
    seoApi
      .verify(target.trim())
      .then(setVerified)
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setBusy(false))
  }
  const schedule = (every: 'daily' | 'weekly') => {
    setNote(null)
    seoApi
      .auditSchedule(target.trim(), depth, every)
      .then(() => setNote(every === 'daily' ? 'Günlük denetim rutini eklendi; her koşu öncekiyle farkı bildirir.' : 'Haftalık denetim rutini eklendi; her koşu öncekiyle farkı bildirir.'))
      .catch((err: unknown) => setNote(errorMessage(err)))
  }

  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`} data-kor-event="kor:seo/progress">
      <h3 id={`${uid}-h`} className="kg-card-title ks-title">
        <ScanSearch size={16} aria-hidden="true" />
        Teknik SEO denetimi
      </h3>
      <form className="kp-form" onSubmit={submit}>
        <label className="kg-field kp-grow">
          <span>Site, sayfa ya da site haritası</span>
          <input value={target} onChange={(e) => setTarget(e.target.value)} placeholder="ornek.com ya da https://ornek.com/sitemap.xml" spellCheck={false} maxLength={500} translate="no" required />
        </label>
        <label className="kg-field">
          <span>Derinlik</span>
          <select value={depth} onChange={(e) => setDepth(e.target.value as SeoDepth)}>
            <option value="quick">Hızlı (25 sayfa)</option>
            <option value="full">Tam (500 sayfa)</option>
            <option value="forensic">Adli (10 000 sayfa)</option>
          </select>
        </label>
        <label className="kseo-check">
          <input type="checkbox" checked={vitals} onChange={(e) => setVitals(e.target.checked)} />
          Core Web Vitals ölç
        </label>
        <button type="submit" className="btn small" disabled={busy || !target.trim()} data-kor-cmd="seo.audit">
          <ScanSearch size={14} aria-hidden="true" />
          {busy ? 'Taranıyor…' : 'Denetle'}
        </button>
        <button type="button" className="btn small ghost" disabled={!target.trim()} onClick={() => schedule('weekly')} data-kor-cmd="seo.auditSchedule">
          <CalendarClock size={14} aria-hidden="true" />
          Haftalık tekrarla
        </button>
        <button type="button" className="btn small ghost" disabled={busy || !target.trim()} onClick={verify} data-kor-cmd="seo.verify">
          <CheckCheck size={14} aria-hidden="true" />
          Düzeltmeleri doğrula
        </button>
      </form>
      <p className="ks-note">
        Kor robots.txt kurallarına ve Crawl-delay değerine uyar, aynı sunucuya iki istek arasında bekler ve kendini kor/sürüm olarak tanıtır. Core Web Vitals başsız tarayıcı gerektirir (kor component add browser); yoksa raporda not düşülür.
      </p>
      {note && <p className="muted kp-small" role="status">{note}</p>}
      {busy && <p className="muted kp-small" role="status" translate="no">{progress || 'Başlıyor…'}</p>}
      {error && <p className="error-text kp-pre" role="alert" translate="no">{error}</p>}
      {verified && (
        <div>
          <p className="kp-small">{`${verified.urls} sayfa yeniden tarandı: ${verified.fixed.length} düzelmiş, ${verified.remaining.length} kalan, ${verified.new.length} yeni bulgu.`}</p>
          {verified.remaining.length + verified.new.length > 0 && (
            <ul className="kp-notes" translate="no">
              {[...verified.remaining, ...verified.new].slice(0, 50).map((f, i) => <li key={i}>{`${f.file}:${f.line} ${f.rule} — ${f.message}`}</li>)}
            </ul>
          )}
        </div>
      )}
      {result && (
        <>
          <p className="kp-small">
            {`${result.pages} sayfa tarandı · ${result.report.findings.length} bulgu`}
            {result.diff && ` · önceki denetimden beri ${result.diff.new.length} yeni, ${result.diff.fixed.length} düzelmiş`}
          </p>
          {result.rich.length > 0 && (
            <details>
              <summary className="kp-h">Zengin sonuç uygunluğu</summary>
              <ul className="kp-notes" translate="no">
                {result.rich.map((r) => (
                  <li key={r.url}>
                    {`${r.url}: ${r.eligible.join(', ') || '—'}`}
                    {r.blocked.length > 0 && ` · eksik: ${r.blocked.map(([t, f]) => `${t} (${f.join(', ')})`).join('; ')}`}
                  </li>
                ))}
              </ul>
            </details>
          )}
          <Findings report={result.report} title={`SEO denetimi: ${target}`} file="seo-audit.html" />
        </>
      )}
    </section>
  )
}
