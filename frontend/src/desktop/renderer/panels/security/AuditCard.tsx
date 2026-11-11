// Denetim kaydı (§6.7): audit.jsonl sayfalı, eylem/model/dosya süzgeci ve hash zinciri doğrulaması.
import { ChevronLeft, ChevronRight, Download, FileClock, FileText, ShieldCheck, ShieldX } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { AuditEntry, AuditVerify } from '../../../ipc-security.ts'
import { errorMessage } from '../../api.ts'
import { CopyButton } from '../../CopyButton.tsx'
import { Markdown } from '../../Markdown.tsx'
import { auditApi } from './bridge.ts'
import { AUDIT_PAGE, decisionLabel, entrySummary, pageInfo, verifyText } from './shape.ts'

const ACTION_LABELS: Record<string, string> = {
  tool: 'Araç çağrısı',
  task: 'Görev (mesaj)',
  model: 'Model çağrısı',
  sandbox_denied: 'Sandbox engeli',
}

/** KVKK/GDPR uyum raporu (§6.7) ve tüm kaydın JSONL kopyası. Rapor içerik değil, yalnızca sayılar ve yollar içerir. */
function ReportSection() {
  const id = useId()
  const [since, setSince] = useState('')
  const [markdown, setMarkdown] = useState<string | null>(null)
  const [exported, setExported] = useState<string | null>(null)
  const [busy, setBusy] = useState<'report' | 'export' | null>(null)
  const [error, setError] = useState<string | null>(null)

  const build = async (e: FormEvent) => {
    e.preventDefault()
    setBusy('report')
    setError(null)
    try {
      setMarkdown((await auditApi.report(since || null)).markdown)
    } catch (err) {
      setMarkdown(null)
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }
  const exportAll = async () => {
    setBusy('export')
    setError(null)
    try {
      const r = await auditApi.export(null)
      setExported(`${r.entries} kayıt şu dosyaya kopyalandı: ${r.path}${r.ok ? '' : ' (doğrulama başarısız: önce zinciri doğrulayın)'}`)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  return (
    <details className="ks-more">
      <summary>Uyum raporu (KVKK / GDPR) ve dışa aktarma</summary>
      <form className="ks-inline" onSubmit={(e) => void build(e)}>
        <label htmlFor={`${id}-s`}>Başlangıç</label>
        <input id={`${id}-s`} className="text-input" type="date" value={since} onChange={(e) => setSince(e.target.value)} />
        <button type="submit" className="btn small" disabled={busy !== null} data-kor-cmd="audit.report">
          <FileText size={14} aria-hidden="true" />
          {busy === 'report' ? 'Hazırlanıyor…' : 'Raporu oluştur'}
        </button>
        <button type="button" className="btn small" onClick={() => void exportAll()} disabled={busy !== null} data-kor-cmd="audit.export">
          <Download size={14} aria-hidden="true" />
          {busy === 'export' ? 'Kopyalanıyor…' : 'JSONL olarak dışa aktar'}
        </button>
      </form>
      {exported && <p className="muted ks-note" role="status" translate="no">{exported}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
      {markdown && (
        <div className="ks-preview" aria-label="Uyum raporu">
          <div className="ks-inline">
            <CopyButton getText={() => markdown} label="Markdown olarak kopyala" />
          </div>
          <Markdown text={markdown} />
        </div>
      )}
    </details>
  )
}

function Entry({ e }: { e: AuditEntry }) {
  const time = new Date(e.ts)
  const bad = e.decision === 'deny' || e.decision === 'rejected' || e.ok === false
  return (
    <li className="ks-entry">
      <div className="ks-entry-head">
        <span className="ks-chip">{ACTION_LABELS[e.action] ?? e.action}</span>
        {e.decision && <span className={`ks-chip${bad ? ' bad' : ''}`}>{decisionLabel(e.decision)}</span>}
        {e.ok === false && <span className="ks-chip bad">hata</span>}
        <time className="muted ks-time" dateTime={e.ts}>{Number.isNaN(time.getTime()) ? e.ts : time.toLocaleString()}</time>
      </div>
      <p className="ks-entry-text" translate="no">{entrySummary(e) || '—'}</p>
      <p className="muted ks-entry-meta" translate="no">
        {[e.model, e.action === 'task' && e.input !== undefined ? `${e.input} → ${e.output ?? 0} token` : null].filter(Boolean).join(' · ')}
      </p>
    </li>
  )
}

function VerifyResult({ v }: { v: AuditVerify }) {
  return (
    <p className={v.ok ? 'ks-ok' : 'ks-broken'} role={v.ok ? 'status' : 'alert'}>
      {v.ok ? <ShieldCheck size={14} aria-hidden="true" /> : <ShieldX size={14} aria-hidden="true" />}
      {verifyText(v)}
    </p>
  )
}

export function AuditCard() {
  const id = useId()
  const [action, setAction] = useState('')
  const [model, setModel] = useState('')
  const [file, setFile] = useState('')
  const [offset, setOffset] = useState(0)
  const query = { action, model, file: file.trim(), offset, limit: AUDIT_PAGE }
  const { data, error, isLoading, mutate } = useSWR(['security:audit', action, model, query.file, offset], () => auditApi.list(query), { keepPreviousData: true })
  const [verify, setVerify] = useState<AuditVerify | null>(null)
  const [verifyError, setVerifyError] = useState<string | null>(null)
  const [verifying, setVerifying] = useState(false)

  const check = async () => {
    setVerifying(true)
    setVerifyError(null)
    try {
      setVerify(await auditApi.verify())
      await mutate()
    } catch (err) {
      setVerify(null)
      setVerifyError(errorMessage(err))
    } finally {
      setVerifying(false)
    }
  }
  const filter = (set: (v: string) => void) => (v: string) => {
    set(v)
    setOffset(0)
  }
  const page = data ? pageInfo(data.total, offset, AUDIT_PAGE) : null

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title">
          <FileClock size={16} aria-hidden="true" />
          Denetim kaydı
        </h3>
        <button type="button" className="btn small" onClick={() => void check()} disabled={verifying} data-kor-cmd="audit.verify">
          <ShieldCheck size={14} aria-hidden="true" />
          {verifying ? 'Doğrulanıyor…' : 'Zinciri doğrula'}
        </button>
      </div>
      {data && <p className="muted ks-note">Ajanın her aracı, onayı, model çağrısı (maliyetiyle) ve görevi hash zinciriyle şu dosyaya eklenir: <code translate="no">{data.path}</code></p>}
      {verify && <VerifyResult v={verify} />}
      {verifyError && <p className="error-text" role="alert">{verifyError}</p>}
      <ReportSection />

      <div className="ks-filters">
        <label htmlFor={`${id}-a`} className="sr-only">Eylem</label>
        <select id={`${id}-a`} className="g7-select" value={action} onChange={(e) => filter(setAction)(e.target.value)}>
          <option value="">Tüm eylemler</option>
          {(data?.actions ?? []).map((a) => (
            <option key={a} value={a}>{ACTION_LABELS[a] ?? a}</option>
          ))}
        </select>
        <label htmlFor={`${id}-m`} className="sr-only">Model</label>
        <select id={`${id}-m`} className="g7-select" value={model} onChange={(e) => filter(setModel)(e.target.value)}>
          <option value="">Tüm modeller</option>
          {(data?.models ?? []).map((m) => (
            <option key={m} value={m} translate="no">{m}</option>
          ))}
        </select>
        <label htmlFor={`${id}-f`} className="sr-only">Dosya ya da komut</label>
        <input id={`${id}-f`} className="text-input" value={file} onChange={(e) => filter(setFile)(e.target.value)} placeholder="Dosya ya da komut içerir…" maxLength={200} spellCheck={false} />
      </div>

      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status" data-kor-cmd="audit.list">Denetim kaydı okunuyor…</p>
      ) : data.total === 0 ? (
        <p className="muted ks-empty" data-kor-cmd="audit.list">
          {action || model || file ? 'Süzgece uyan kayıt yok.' : 'Henüz kayıt yok. Ajan bir araç çalıştırdığında ya da bir görevi bitirdiğinde burada görünür.'}
        </p>
      ) : (
        <>
          <ul className="ks-entries" aria-label="Denetim kayıtları" aria-busy={isLoading} data-kor-cmd="audit.list">
            {data.entries.map((e) => (
              <Entry key={e.hash} e={e} />
            ))}
          </ul>
          {page && (
            <div className="ks-pager">
              <button type="button" className="action-btn" onClick={() => setOffset(Math.max(0, offset - AUDIT_PAGE))} disabled={!page.hasPrev} aria-label="Daha yeni kayıtlar" title="Daha yeni">
                <ChevronLeft size={14} aria-hidden="true" />
              </button>
              <span className="muted">{`${page.from}–${page.to} / ${data.total}`}</span>
              <button type="button" className="action-btn" onClick={() => setOffset(offset + AUDIT_PAGE)} disabled={!page.hasNext} aria-label="Daha eski kayıtlar" title="Daha eski">
                <ChevronRight size={14} aria-hidden="true" />
              </button>
            </div>
          )}
        </>
      )}
    </section>
  )
}
