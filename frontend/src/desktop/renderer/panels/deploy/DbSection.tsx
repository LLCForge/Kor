// §8.5 veritabanı ekleri: index önerileri, ajanla yeni migration, SQL'i önce kopyada deneyip uygulama.
import { Database, FilePlus2, FlaskConical, Play } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import useSWR from 'swr'
import { errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { CopyButton } from '../../CopyButton.tsx'
import { insertIntoDraft } from '../../g21/quote.ts'
import { dbOpsApi } from './bridge.ts'

export function DbSection() {
  const idx = useSWR('db:indexes', () => dbOpsApi.indexes(), { revalidateOnFocus: false })
  const [desc, setDesc] = useState('')
  const [conn, setConn] = useState('')
  const [sql, setSql] = useState('')
  const [change, setChange] = useState<string[] | null>(null)
  const [msg, setMsg] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const guard = async (f: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    setMsg(null)
    try {
      await f()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const migrate = (e: FormEvent) => {
    e.preventDefault()
    void guard(async () => {
      const { style, prompt } = await dbOpsApi.migrateNew(desc.trim())
      insertIntoDraft(prompt)
      setMsg(`Görev mesaj kutusuna yazıldı (${style.framework}, ${style.dir}/). Göndererek ajanı başlatın.`)
      setDesc('')
    })
  }

  const apply = () =>
    guard(async () => {
      const destructive = await dbOpsApi.check(sql)
      if (destructive.length > 0) {
        const tryIt = await confirmAction({
          title: 'Yıkıcı SQL',
          message: `${destructive.length} yıkıcı ifade var:\n${destructive.join('\n')}\n\nÖnce veritabanının bir kopyasında denenecek.`,
          confirmLabel: 'Kopyada dene',
          danger: true,
        })
        if (!tryIt) return
        const c = await dbOpsApi.trial(conn.trim(), sql)
        setChange(c)
      }
      const ok = await confirmAction({
        title: `${conn.trim()} üzerinde çalıştır`,
        message: destructive.length > 0 ? 'Kopya hatasız çalıştırdı. Şimdi gerçek veritabanına uygulansın mı? Geri alınamaz.' : 'SQL gerçek veritabanında çalışacak.',
        confirmLabel: 'Uygula',
        danger: destructive.length > 0,
      })
      if (!ok) return
      await dbOpsApi.apply(conn.trim(), sql)
      setMsg(`${conn.trim()} üzerine uygulandı.`)
    })

  return (
    <section className="kg-card" aria-label="Veritabanı">
      <h4 className="kd-h">
        <Database size={14} aria-hidden="true" />
        Veritabanı
      </h4>
      {idx.error ? (
        <p className="error-text" role="alert">{errorMessage(idx.error)}</p>
      ) : !idx.data ? (
        <p className="muted" role="status" data-kor-cmd="db.indexes">Index'ler okunuyor…</p>
      ) : idx.data.tables.length === 0 && idx.data.suggestions.length === 0 ? (
        <p className="muted" data-kor-cmd="db.indexes">Bağlı veritabanı yok ya da index bilgisi yok (kor db connect).</p>
      ) : (
        <div data-kor-cmd="db.indexes">
          <p className="kd-small">{`${idx.data.suggestions.length} index önerisi`}</p>
          <ul className="kd-cmds">
            {idx.data.suggestions.map((s) => (
              <li key={`${s.connection}.${s.table}.${s.columns.join(',')}`}>
                <code translate="no">{s.sql}</code>
                <CopyButton getText={() => s.sql} />
                <span className="muted kd-small" translate="no">{s.reason}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      <form className="kg-row" onSubmit={migrate}>
        <label className="kg-field kd-grow">
          <span>Yeni migration</span>
          <input value={desc} onChange={(e) => setDesc(e.target.value)} placeholder="users tablosuna email_verified ekle" maxLength={300} required />
        </label>
        <button type="submit" className="btn small" disabled={busy} data-kor-cmd="db.migrateNew">
          <FilePlus2 size={14} aria-hidden="true" />
          Ajana yazdır
        </button>
      </form>
      <div className="kd-sql">
        <label className="kg-field">
          <span>Bağlantı</span>
          <input value={conn} onChange={(e) => setConn(e.target.value)} placeholder="main" spellCheck={false} maxLength={40} translate="no" />
        </label>
        <label className="kg-field">
          <span>SQL</span>
          <textarea value={sql} onChange={(e) => setSql(e.target.value)} rows={4} spellCheck={false} translate="no" />
        </label>
        <button type="button" className="btn small" disabled={busy || !conn.trim() || !sql.trim()} onClick={() => void apply()} data-kor-cmd="db.apply">
          {busy ? <FlaskConical size={14} aria-hidden="true" /> : <Play size={14} aria-hidden="true" />}
          {busy ? 'Çalışıyor…' : 'Uygula (yıkıcıysa önce kopyada)'}
        </button>
        <span hidden data-kor-cmd="db.check" />
        <span hidden data-kor-cmd="db.trial" />
      </div>
      {change && (
        <div>
          <p className="kd-small">Kopyadaki şema değişikliği:</p>
          <pre className="kd-out" translate="no">{change.length ? change.join('\n') : '(şema değişmedi)'}</pre>
        </div>
      )}
      {msg && <p className="kd-small" role="status">{msg}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}
