// cdesktop "Logs" paneli: araç çağrısı günlüğü (zaman, araç, karar, süre, argüman) + "Günlüklerde ara".
import { RefreshCw, Search, SquareTerminal, Wrench, X } from 'lucide-react'
import { useMemo, useState } from 'react'
import useSWR from 'swr'
import { matchesLogQuery } from '../audit-read.ts'
import type { LogEntry } from '../ipc-logs.ts'
import { api, errorMessage } from './api.ts'
import { uiLocale } from './ui-locale.ts'

const DECISION_LABEL: Record<LogEntry['decision'], string> = { allow: 'izinli', approved: 'onaylandı', deny: 'engellendi', rejected: 'reddedildi', invalid: 'geçersiz' }

function timeOf(ts: string): string {
  const d = new Date(ts)
  return Number.isNaN(d.getTime()) ? ts : d.toLocaleTimeString(uiLocale(), { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

export function LogsPanel({ sessionId }: { sessionId: string | null }) {
  const { data, error, isLoading, mutate } = useSWR(['logs', sessionId], () => api.logs.list(sessionId, 500), { revalidateOnFocus: false })
  const [query, setQuery] = useState('')
  const rows = useMemo(() => (data ?? []).filter((e) => matchesLogQuery(e, query)).reverse(), [data, query])

  return (
    <div className="cdx-logs">
      <div className="cdx-changes-tools">
        <label className="cdx-search">
          <Search size={13} aria-hidden="true" />
          <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Günlüklerde ara" aria-label="Günlüklerde ara" spellCheck={false} />
          {query && (
            <button type="button" className="action-btn" onClick={() => setQuery('')} aria-label="Aramayı temizle">
              <X size={12} aria-hidden="true" />
            </button>
          )}
        </label>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Yenile" title="Yenile">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {error ? (
        <p className="error-text pad">{errorMessage(error)}</p>
      ) : isLoading ? (
        <p className="muted pad">Günlük okunuyor…</p>
      ) : rows.length === 0 ? (
        <p className="muted pad">{query ? `“${query}” ile eşleşen kayıt yok.` : 'Bu sohbet için henüz araç çağrısı günlüğü yok.'}</p>
      ) : (
        <ul className="cdx-log-list" aria-label="Araç çağrısı günlüğü">
          {rows.map((e, i) => (
            <li key={`${e.ts}-${i}`} className={`cdx-log-row ${e.decision}${e.ok === false ? ' failed' : ''}`}>
              <span className="cdx-log-icon" aria-hidden="true">{e.tool === 'bash' ? <SquareTerminal size={13} /> : <Wrench size={13} />}</span>
              <span className="cdx-log-main">
                <span className="cdx-log-head">
                  <span className="cdx-log-tool">{e.tool}</span>
                  <span className={`badge neutral cdx-log-decision ${e.decision}`}>{DECISION_LABEL[e.decision]}</span>
                  {e.ok === false && <span className="badge neutral cdx-log-fail">hata</span>}
                  <span className="cdx-log-time">
                    {timeOf(e.ts)}
                    {typeof e.durationMs === 'number' && ` · ${e.durationMs < 1000 ? `${e.durationMs} ms` : `${(e.durationMs / 1000).toFixed(1)} sn`}`}
                  </span>
                </span>
                <code className="cdx-log-args" title={e.args}>{e.args}</code>
                {e.reason && <span className="cdx-log-reason">{e.reason}</span>}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
