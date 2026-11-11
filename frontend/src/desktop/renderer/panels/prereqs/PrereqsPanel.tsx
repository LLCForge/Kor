// Sistem gereksinimleri görünümü (§24.3 Katman C, ADR 0064): Kor'un kendi klasörüne kuramadığı şeyler
// (Docker/Podman, WSL2, Xcode araçları, WebView2) ve macOS izinleri. Her satır tam komutu gösterir; "Kur"
// yerel bir onay penceresi açar, şifreyi Kor değil sistem sorar. Reddedilirse Kor çalışmaya devam eder.
import { CheckCircle2, CircleAlert, PackageCheck, RefreshCw, Settings2, Terminal } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import type { PrereqId, PrereqRow } from '../../../ipc-prereq.ts'
import { errorMessage } from '../../api.ts'
import type { KorPanelProps } from '../registry.ts'
import { prereqApi } from './bridge.ts'
import { LABEL, actionText, appendLine, reasonText, resultText, statusText } from './shape.ts'

const KEY = 'prereq:list'

function Row({ row, busy, log, onInstall }: { row: PrereqRow; busy: boolean; log: string[]; onInstall: (id: PrereqId) => void }) {
  const id = useId()
  const label = LABEL[row.id]
  const action = actionText(row)
  const reason = reasonText(row)
  return (
    <li className="kp-row" aria-labelledby={`${id}-n`}>
      <div className="kp-head">
        {row.installed === true ? <CheckCircle2 size={16} aria-hidden="true" className="kp-ok" /> : <CircleAlert size={16} aria-hidden="true" className="kp-miss" />}
        <span id={`${id}-n`} className="kp-name">{label.name}</span>
        <span className={`ks-badge${row.installed === true ? ' on' : ''}`}>{statusText(row)}</span>
      </div>
      <p className="muted kp-why">{label.why}</p>
      {row.command && row.installed !== true && (
        <p className="kp-cmd">
          <Terminal size={12} aria-hidden="true" />
          <code translate="no">{row.command}</code>
        </p>
      )}
      {reason && <p className="ks-warn">{reason}</p>}
      {action && (
        <div className="ko-actions">
          <button type="button" className="btn small" disabled={busy} onClick={() => onInstall(row.id)} data-kor-cmd="prereq.install">
            {row.action === 'open-settings' ? <Settings2 size={14} aria-hidden="true" /> : <PackageCheck size={14} aria-hidden="true" />}
            {busy ? 'Çalışıyor…' : action}
          </button>
        </div>
      )}
      {log.length > 0 && (
        <pre className="kp-log" translate="no" aria-live="polite">{log.join('\n')}</pre>
      )}
    </li>
  )
}

export function PrereqsPanel(_: KorPanelProps) {
  const { data, error, isLoading, mutate } = useSWR(KEY, () => prereqApi.list(), { revalidateOnFocus: false })
  const [busy, setBusy] = useState<PrereqId | null>(null)
  const [log, setLog] = useState<Record<string, string[]>>({})
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null)

  useEffect(() => prereqApi.onProgress((e) => setLog((l) => appendLine(l, e.id, e.line))), [])

  const install = async (id: PrereqId) => {
    setBusy(id)
    setMessage(null)
    setLog((l) => ({ ...l, [id]: [] }))
    try {
      const r = await prereqApi.install(id)
      setMessage({ ok: r.ok, text: resultText(id, r) })
      await mutate((rows) => rows?.map((x) => (x.id === id ? r.row : x)), { revalidate: false })
    } catch (err) {
      setMessage({ ok: false, text: errorMessage(err) })
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="ks" data-kor-event="kor:prereq/progress">
      <div className="kg-card-head">
        <h2 className="ks-title">Sistem gereksinimleri</h2>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Sistem gereksinimlerini yeniden denetle" title="Yenile" data-kor-cmd="prereq.list">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      <p className="muted ks-note">
        Kor gereken her şeyi önce kendi klasörüne kurar. Yalnızca işletim sisteminin kurabildikleri burada: ne gerektiği, neden gerektiği ve
        çalışacak tam komut. Siz onaylamadan hiçbir şey çalışmaz; şifreyi Kor değil sistem sorar.
      </p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : isLoading || !data ? (
        <p className="muted" role="status" data-kor-cmd="prereq.list">Sistem denetleniyor…</p>
      ) : data.length === 0 ? (
        <p className="ks-empty">Bu sistemde Kor'un işletim sistemi düzeyinde ihtiyaç duyabileceği bir şey yok.</p>
      ) : (
        <ul className="kp-list">
          {data.map((row) => (
            <Row key={row.id} row={row} busy={busy === row.id} log={log[row.id] ?? []} onInstall={(id) => void install(id)} />
          ))}
        </ul>
      )}
      {message && (message.ok ? <p className="ks-ok" role="status">{message.text}</p> : <p className="ks-broken" role="alert">{message.text}</p>)}
    </div>
  )
}
