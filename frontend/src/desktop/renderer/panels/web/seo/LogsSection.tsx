// Log dosyası SEO analizi (§29.9): Apache/nginx combined ve JSON satırları. Botlar kullanıcı ajanından tanınır;
// ters DNS doğrulaması yalnız istenince ve ağ açıkken yapılır, yoksa "doğrulanmamış" sayılır.
import { FileText, FolderOpen } from 'lucide-react'
import { useId, useState } from 'react'
import type { SeoLogsResult } from '../../../../ipc-seo.ts'
import { api, errorMessage } from '../../../api.ts'
import { seoApi } from './api.ts'
import { Findings } from './Findings.tsx'

export function LogsSection() {
  const uid = useId()
  const [paths, setPaths] = useState<string[]>([])
  const [sitemap, setSitemap] = useState('')
  const [verifyDns, setVerifyDns] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<SeoLogsResult | null>(null)

  const pick = () => {
    api.pickFiles().then((p) => p.length && setPaths(p)).catch((e: unknown) => setError(errorMessage(e)))
  }
  const run = () => {
    setBusy(true)
    setError(null)
    seoApi
      .logs(paths, sitemap.trim() || null, verifyDns)
      .then(setResult)
      .catch((e: unknown) => setError(errorMessage(e)))
      .finally(() => setBusy(false))
  }

  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <h3 id={`${uid}-h`} className="kg-card-title ks-title">
        <FileText size={16} aria-hidden="true" />
        Log dosyası SEO analizi
      </h3>
      <div className="kp-form">
        <button type="button" className="btn small ghost" onClick={pick}>
          <FolderOpen size={14} aria-hidden="true" />
          Log dosyası seç
        </button>
        <label className="kg-field kp-grow">
          <span>Site haritası (hiç taranmayan URL'ler için, isteğe bağlı)</span>
          <input value={sitemap} onChange={(e) => setSitemap(e.target.value)} placeholder="https://ornek.com/sitemap.xml" spellCheck={false} translate="no" />
        </label>
        <label className="kseo-check">
          <input type="checkbox" checked={verifyDns} onChange={(e) => setVerifyDns(e.target.checked)} />
          Ters DNS ile doğrula
        </label>
        <button type="button" className="btn small" disabled={busy || paths.length === 0} onClick={run} data-kor-cmd="seo.logs">
          {busy ? 'Okunuyor…' : 'Analiz et'}
        </button>
      </div>
      {paths.length > 0 && <p className="muted kp-small kp-loc" translate="no">{paths.join(', ')}</p>}
      {error && <p className="error-text kp-pre" role="alert" translate="no">{error}</p>}
      {result && (
        <>
          {result.files.map(({ file, stats }) => (
            <div key={file} className="kp-table-wrap">
              <table className="kp-table">
                <caption className="kp-caption" translate="no">{`${file}: ${stats.parsed}/${stats.lines} satır${stats.first ? ` · ${stats.first.slice(0, 10)} … ${stats.last?.slice(0, 10) ?? ''}` : ''}`}</caption>
                <thead><tr><th scope="col">Bot</th><th scope="col">İstek</th><th scope="col">Doğrulanan</th><th scope="col">Sahte</th><th scope="col">Doğrulanmamış</th></tr></thead>
                <tbody>
                  {Object.entries(stats.bots).map(([name, b]) => (
                    <tr key={name}>
                      <td translate="no">{name}</td>
                      <td className="kp-num">{b.hits}</td>
                      <td className="kp-num">{b.verified}</td>
                      <td className="kp-num">{b.fake}</td>
                      <td className="kp-num">{b.unverified}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <p className="kp-small">
                {`Googlebot tarama bütçesi: parametreli ${stats.waste.parameter_urls ?? 0}% · yönlendirme ${stats.waste.redirects ?? 0}% · 4xx ${stats.waste.client_errors ?? 0}% · 5xx ${stats.waste.server_errors ?? 0}%`}
                {stats.p95_ms !== null && ` · yanıt p50 ${stats.p50_ms?.toFixed(0)} ms, p95 ${stats.p95_ms.toFixed(0)} ms`}
              </p>
            </div>
          ))}
          <Findings report={result.report} title="Log dosyası SEO analizi" file="seo-logs.html" />
        </>
      )}
    </section>
  )
}
