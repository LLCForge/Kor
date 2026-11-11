// Bekleyen onay (§18.3): araç, özet, yıkıcılık uyarısı ve dosya başına diff; dosya başına kabul/ret (ADR 0030).
import { useState } from 'react'
import { client } from './bus.ts'
import { DiffView, type FileChange } from './DiffView.tsx'
import { t } from './i18n.ts'

export interface ConfirmEvent {
  id: string
  name: string
  summary: string
  warning?: string
  destructive?: string | null
  files?: FileChange[]
}

export function Approval({ ev, onDone }: { ev: ConfirmEvent; onDone: () => void }) {
  const files = ev.files ?? []
  const [decisions, setDecisions] = useState<Record<string, 'accept' | 'reject'>>(() => Object.fromEntries(files.map((f) => [f.path, 'accept'])))
  const [error, setError] = useState('')
  const answer = (approval: 'yes' | 'no' | 'always') => {
    const body: Record<string, unknown> = { id: ev.id, approval }
    if (files.length > 0 && approval !== 'no') body.files = decisions
    client('approve', body).then(onDone, (e: Error) => setError(e.message))
  }
  return (
    <div className={`confirm${ev.destructive ? ' danger' : ''}`} data-kor-cmd="client.approve">
      <div className="q">
        <span translate="no">{ev.name}</span>: <span translate="no">{ev.summary}</span>
      </div>
      {ev.warning ? <div className="warn">{ev.warning}</div> : null}
      {ev.destructive ? <div className="warn">{t('Geri alınamaz işlem')}: {ev.destructive}</div> : null}
      {files.map((f) => (
        <div key={f.path} className="file-review">
          <DiffView file={f} />
          <div className="row">
            <label className="sw">
              <input
                type="checkbox"
                checked={decisions[f.path] === 'accept'}
                onChange={(e) => setDecisions({ ...decisions, [f.path]: e.target.checked ? 'accept' : 'reject' })}
              />
              {t('Bu dosyayı kabul et')}
            </label>
          </div>
        </div>
      ))}
      <div className="row">
        <button type="button" className="primary" onClick={() => answer('yes')}>
          {t('İzin ver')}
        </button>
        {ev.destructive ? null : (
          <button type="button" onClick={() => answer('always')}>
            {t('Her zaman')}
          </button>
        )}
        <button type="button" onClick={() => answer('no')}>
          {t('Reddet')}
        </button>
      </div>
      {error ? <div className="err">{error}</div> : null}
    </div>
  )
}
