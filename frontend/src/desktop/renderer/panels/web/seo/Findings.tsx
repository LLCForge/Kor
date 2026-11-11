// Bulgu tablosu + dışa aktarma (HTML/PDF/SARIF/JSON/Markdown). Bulgu metinleri sitenin içeriğinden gelir:
// translate="no".
import { Download } from 'lucide-react'
import { useId, useState } from 'react'
import type { SeoReport, SeoSeverity } from '../../../../ipc-seo.ts'
import { errorMessage } from '../../../api.ts'
import { seoApi } from './api.ts'
import { SEVERITIES, SEVERITY_LABEL, counts, filterFindings, location } from './format.ts'

const LIMIT = 300

export function Findings({ report, title, file }: { report: SeoReport; title: string; file: string }) {
  const uid = useId()
  const [min, setMin] = useState<SeoSeverity>('low')
  const [q, setQ] = useState('')
  const [format, setFormat] = useState<'html' | 'pdf' | 'sarif' | 'json' | 'markdown'>('html')
  const [path, setPath] = useState(`.kor/seo/${file}`)
  const [brand, setBrand] = useState('')
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)
  const [busy, setBusy] = useState(false)
  const c = counts(report.findings)
  const rows = filterFindings(report.findings, min, q)
  const ext = format === 'markdown' ? 'md' : format
  const exportReport = () => {
    setBusy(true)
    setNote(null)
    const target = path.replace(/\.[a-z]+$/i, '') + `.${ext}`
    seoApi
      .exportReport(report, format, target, title, brand.trim() || null)
      .then((p) => setNote({ text: `Yazıldı: ${p}`, ok: true }))
      .catch((e: unknown) => setNote({ text: errorMessage(e), ok: false }))
      .finally(() => setBusy(false))
  }
  return (
    <div className="kseo-findings">
      <p className="kp-small" aria-label="Önem dağılımı">
        {SEVERITIES.map((s) => `${SEVERITY_LABEL[s]} ${c[s]}`).join(' · ')}
      </p>
      <div className="kp-form">
        <label className="kg-field">
          <span>En az</span>
          <select value={min} onChange={(e) => setMin(e.target.value as SeoSeverity)}>
            {SEVERITIES.map((s) => <option key={s} value={s}>{SEVERITY_LABEL[s]}</option>)}
          </select>
        </label>
        <label className="kg-field kp-grow">
          <span>Süz</span>
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="URL, kural ya da metin" spellCheck={false} maxLength={200} />
        </label>
      </div>
      {rows.length === 0 ? (
        <p className="ks-empty">Bu süzgeçle bulgu yok.</p>
      ) : (
        <div className="kp-table-wrap">
          <table className="kp-table">
            <caption className="sr-only">{title}</caption>
            <thead>
              <tr>
                <th scope="col">Önem</th>
                <th scope="col">Konum</th>
                <th scope="col">Kural</th>
                <th scope="col">Bulgu ve düzeltme</th>
              </tr>
            </thead>
            <tbody>
              {rows.slice(0, LIMIT).map((f, i) => (
                <tr key={`${f.file}-${f.line}-${f.rule}-${i}`} className={`kseo-${f.severity}`}>
                  <td>{SEVERITY_LABEL[f.severity]}</td>
                  <td className="kp-loc" translate="no">{location(f)}</td>
                  <td className="kp-fn" translate="no">{f.rule}</td>
                  <td translate="no">
                    {f.message}
                    {f.fix && <div className="muted kseo-fix">{`→ ${f.fix}`}</div>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length > LIMIT && <p className="muted kp-small">{`İlk ${LIMIT} bulgu gösteriliyor (${rows.length}); tamamı dışa aktarımda.`}</p>}
        </div>
      )}
      {report.notes.length > 0 && (
        <ul className="kp-notes" aria-label="Notlar" translate="no">
          {report.notes.map((n, i) => <li key={i}>{n}</li>)}
        </ul>
      )}
      <form className="kp-form" aria-labelledby={`${uid}-x`} onSubmit={(e) => { e.preventDefault(); exportReport() }}>
        <span id={`${uid}-x`} className="sr-only">Raporu dışa aktar</span>
        <label className="kg-field">
          <span>Biçim</span>
          <select value={format} onChange={(e) => setFormat(e.target.value as typeof format)}>
            <option value="html">HTML</option>
            <option value="pdf">PDF</option>
            <option value="sarif">SARIF</option>
            <option value="json">JSON</option>
            <option value="markdown">Markdown</option>
          </select>
        </label>
        <label className="kg-field kp-grow">
          <span>Dosya</span>
          <input value={path} onChange={(e) => setPath(e.target.value)} spellCheck={false} maxLength={500} translate="no" />
        </label>
        <label className="kg-field">
          <span>Marka satırı</span>
          <input value={brand} onChange={(e) => setBrand(e.target.value)} maxLength={120} placeholder="Ajans adı" />
        </label>
        <button type="submit" className="btn small" disabled={busy || !path.trim()} data-kor-cmd="seo.exportReport">
          <Download size={14} aria-hidden="true" />
          {busy ? 'Yazılıyor…' : 'Dışa aktar'}
        </button>
      </form>
      {note && <p className={note.ok ? 'muted kp-small' : 'error-text kp-small'} role={note.ok ? 'status' : 'alert'} translate="no">{note.text}</p>}
    </div>
  )
}
