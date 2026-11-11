import { AlertCircle, AlertTriangle, Play } from 'lucide-react'
import useSWR from 'swr'
import type { Diagnostic, ToolReport } from '../ipc-g4.ts'
import { api, errorMessage } from './api.ts'

const TOOL_LABEL = { tsc: 'TypeScript (tsc --noEmit)', eslint: 'ESLint' } as const

function ToolSection({ report, onOpen }: { report: ToolReport; onOpen: (d: Diagnostic) => void }) {
  const errors = report.diagnostics.filter((d) => d.severity === 'error').length
  const warnings = report.diagnostics.length - errors
  return (
    <section className="g4-diag-section" aria-label={TOOL_LABEL[report.tool]}>
      <h3 className="g4-diag-title">
        <span>{TOOL_LABEL[report.tool]}</span>
        {report.status === 'ok' && (
          <span className="g4-diag-counts">
            <span className="stat-del">{errors} hata</span> · <span className="g4-warn">{warnings} uyarı</span> · {(report.durationMs / 1000).toFixed(1)} sn
          </span>
        )}
      </h3>
      {report.status === 'skipped' && <p className="muted g4-diag-note">Atlandı: {report.reason}</p>}
      {report.status === 'failed' && <p className="error-text g4-diag-note" role="alert">Çalıştırılamadı: {report.reason}</p>}
      {report.status === 'ok' && report.diagnostics.length === 0 && <p className="g4-diag-note stat-add">Sorun bulunmadı.</p>}
      {report.diagnostics.length > 0 && (
        <ul className="g4-diag-list">
          {report.diagnostics.map((d, i) => (
            <li key={i}>
              <button type="button" className="g4-diag-row" onClick={() => onOpen(d)} disabled={!d.inWorkspace} title={d.inWorkspace ? 'Dosyayı bu satırda aç' : undefined}>
                {d.severity === 'error' ? (
                  <AlertCircle size={14} className="stat-del" aria-label="hata" />
                ) : (
                  <AlertTriangle size={14} className="g4-warn" aria-label="uyarı" />
                )}
                <span className="g4-diag-text">
                  <span className="g4-diag-loc">
                    {d.file || 'genel'}
                    {d.line > 0 ? `:${d.line}:${d.column}` : ''}
                    {d.code ? ` · ${d.code}` : ''}
                  </span>
                  <span className="g4-diag-msg">{d.message}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {report.truncated && <p className="muted g4-diag-note">Liste kısaltıldı; ilk {report.diagnostics.length} sonuç gösteriliyor.</p>}
    </section>
  )
}

export function Diagnostics({ onOpenFile }: { onOpenFile: (path: string, line?: number) => void }) {
  // Sonuç SWR önbelleğinde kalır: panel yeniden kurulunca (dosya açınca) liste kaybolmaz. Yalnızca istek üzerine çalışır.
  const { data, error, isValidating, mutate } = useSWR('g4:diagnostics', () => api.g4.runDiagnostics(), {
    revalidateOnMount: false,
    revalidateOnFocus: false,
    revalidateOnReconnect: false,
    revalidateIfStale: false,
    shouldRetryOnError: false,
  })

  return (
    <div className="panel-body">
      <div className="panel-file-head">
        <span className="g4-meta">Projedeki yerel tsc ve ESLint çalıştırılır (yapılandırma dosyaları kod çalıştırabilir).</span>
        <button type="button" className="btn small primary" onClick={() => void mutate()} disabled={isValidating}>
          <Play size={14} aria-hidden="true" /> {isValidating ? 'Çalışıyor...' : data ? 'Yeniden çalıştır' : 'Çalıştır'}
        </button>
      </div>
      <div className="g4-diag-body" aria-busy={isValidating}>
        {error && <p className="error-text pad" role="alert">{errorMessage(error)}</p>}
        {!data && !error && !isValidating && (
          <p className="muted pad">
            Tür ve lint hatalarını görmek için Çalıştır'a bas. tsconfig.json varsa <code>tsc --noEmit</code>, ESLint yapılandırması varsa{' '}
            <code>eslint --format json</code> çalışır; sonuçlara tıklayınca dosya o satırda açılır.
          </p>
        )}
        {data && (
          <>
            <ToolSection report={data.tsc} onOpen={(d) => onOpenFile(d.file, d.line || undefined)} />
            <ToolSection report={data.eslint} onOpen={(d) => onOpenFile(d.file, d.line || undefined)} />
          </>
        )}
      </div>
    </div>
  )
}
