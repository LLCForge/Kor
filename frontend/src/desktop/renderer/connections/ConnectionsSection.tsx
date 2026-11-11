// [g31] Ayarlar > Uzak bağlantılar: SSH sunucuları ve veritabanları. Ajan bunlara yalnızca kimlikle erişir (ssh_*, db_* araçları).
import { Database, Pencil, Plus, Server, ShieldAlert, ShieldCheck, Trash2 } from 'lucide-react'
import { useId, useState, type FormEvent, type ReactNode } from 'react'
import useSWR from 'swr'
import type { PublicDbConnection, PublicSshHost } from '../../../core/connections.ts'
import type { ConnectionsView, ConnectionTestResult, DbConnectionInput, SshHostInput } from '../../ipc-connections.ts'
import { api, errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'

const KEY = 'g31:connections'
const ID_HINT = 'küçük harf, rakam, - ve _'
const toId = (text: string) => text.toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40)

type Setter = (view: ConnectionsView) => void

function TestResult({ result, onTrust }: { result: ConnectionTestResult | null; onTrust?: (fingerprint: string) => void }) {
  if (!result) return null
  if (result.ok) return <p className="g31-ok" role="status">Bağlantı başarılı.</p>
  if ('error' in result) return <p className="error-text g9-small" role="alert">{result.error}</p>
  const { fingerprint, mismatch } = result.hostKey
  return (
    <div className={`g31-hostkey${mismatch ? ' danger' : ''}`} role="alert">
      <p>
        {mismatch ? (
          <><ShieldAlert size={14} aria-hidden="true" /> <strong>Sunucu anahtarı değişmiş.</strong> Sunucu yeniden kurulmadıysa bu bir ortadaki adam saldırısı olabilir.</>
        ) : (
          <><ShieldCheck size={14} aria-hidden="true" /> İlk bağlantı: sunucunun kimliğini doğrula.</>
        )}
      </p>
      <code className="g7-mono">{fingerprint}</code>
      <p className="g7-note">
        Sunucuda <code>ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub</code> çıktısıyla aynı olmalı.
      </p>
      {onTrust && (
        <div className="g7-button-row">
          <button type="button" className={`btn small${mismatch ? ' danger' : ''}`} onClick={() => onTrust(fingerprint)}>
            {mismatch ? 'Yeni anahtara güven' : 'Parmak izine güven'}
          </button>
        </div>
      )}
    </div>
  )
}

function SshForm({ initial, canStoreSecrets, onSaved, onError }: { initial?: PublicSshHost; canStoreSecrets: boolean; onSaved: Setter; onError: (m: string) => void }) {
  const fid = useId()
  const editing = initial !== undefined
  const [name, setName] = useState(initial?.name ?? '')
  const [id, setId] = useState(initial?.id ?? '')
  const [host, setHost] = useState(initial?.host ?? '')
  const [port, setPort] = useState(String(initial?.port ?? 22))
  const [username, setUsername] = useState(initial?.username ?? '')
  const [auth, setAuth] = useState<SshHostInput['auth']>(initial?.auth ?? (canStoreSecrets ? 'password' : 'key'))
  const [keyPath, setKeyPath] = useState(initial?.keyPath ?? '~/.ssh/id_ed25519')
  const [password, setPassword] = useState('')
  const effectiveId = id || toId(name)

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const input: SshHostInput = {
      id: effectiveId,
      name: name.trim(),
      host: host.trim(),
      port: Number(port) || 22,
      username: username.trim(),
      auth,
      ...(auth === 'key' ? { keyPath: keyPath.trim() } : {}),
      ...(password && auth !== 'agent' ? { password } : {}),
      ...(auth === 'agent' ? { clearSecret: true } : {}),
    }
    api.connections
      .saveSsh(input)
      .then((view) => {
        setPassword('')
        if (!editing) {
          setName('')
          setId('')
          setHost('')
          setUsername('')
        }
        onSaved(view)
      })
      .catch((err: unknown) => onError(errorMessage(err)))
  }

  const secretLabel = auth === 'password' ? 'Parola' : 'Anahtar parolası (varsa)'
  return (
    <form className="g9-form" onSubmit={submit} autoComplete="off">
      <label htmlFor={`${fid}-n`}>Ad</label>
      <input id={`${fid}-n`} className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Üretim sunucusu" maxLength={80} required />
      <label htmlFor={`${fid}-i`}>Kimlik</label>
      <input id={`${fid}-i`} className="text-input" value={effectiveId} onChange={(e) => setId(toId(e.target.value))} placeholder={ID_HINT} disabled={editing} spellCheck={false} required />
      <label htmlFor={`${fid}-h`}>Sunucu adresi</label>
      <input id={`${fid}-h`} className="text-input" value={host} onChange={(e) => setHost(e.target.value)} placeholder="203.0.113.10 veya sunucu.example.com" spellCheck={false} required />
      <label htmlFor={`${fid}-p`}>Port</label>
      <input id={`${fid}-p`} className="text-input" type="number" min={1} max={65535} value={port} onChange={(e) => setPort(e.target.value)} />
      <label htmlFor={`${fid}-u`}>Kullanıcı</label>
      <input id={`${fid}-u`} className="text-input" value={username} onChange={(e) => setUsername(e.target.value)} placeholder="root, ubuntu, deploy" spellCheck={false} required />
      <label htmlFor={`${fid}-a`}>Kimlik doğrulama</label>
      <select id={`${fid}-a`} className="g7-select" value={auth} onChange={(e) => setAuth(e.target.value as SshHostInput['auth'])}>
        <option value="password" disabled={!canStoreSecrets}>Parola</option>
        <option value="key">Özel anahtar dosyası</option>
        <option value="agent">ssh-agent</option>
      </select>
      {auth === 'key' && (
        <>
          <label htmlFor={`${fid}-k`}>Anahtar dosyası</label>
          <input id={`${fid}-k`} className="text-input" value={keyPath} onChange={(e) => setKeyPath(e.target.value)} spellCheck={false} required />
        </>
      )}
      {auth !== 'agent' && canStoreSecrets && (
        <>
          <label htmlFor={`${fid}-s`}>{secretLabel}</label>
          <input
            id={`${fid}-s`}
            className="text-input"
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={initial?.hasSecret ? 'Kayıtlı · değiştirmek için yaz' : ''}
            autoComplete="new-password"
            required={auth === 'password' && !initial?.hasSecret}
          />
        </>
      )}
      <div className="g7-button-row">
        <button type="submit" className="btn small" disabled={!effectiveId || !name.trim() || !host.trim() || !username.trim()}>
          {editing ? 'Kaydet' : (<><Plus size={14} aria-hidden="true" /> Sunucu ekle</>)}
        </button>
      </div>
    </form>
  )
}

function DbForm({ initial, sshHosts, canStoreSecrets, onSaved, onError }: { initial?: PublicDbConnection; sshHosts: readonly PublicSshHost[]; canStoreSecrets: boolean; onSaved: Setter; onError: (m: string) => void }) {
  const fid = useId()
  const editing = initial !== undefined
  const [name, setName] = useState(initial?.name ?? '')
  const [id, setId] = useState(initial?.id ?? '')
  const [kind, setKind] = useState<DbConnectionInput['kind']>(initial?.kind ?? 'postgres')
  const [host, setHost] = useState(initial?.host ?? '127.0.0.1')
  const [port, setPort] = useState(initial?.port ? String(initial.port) : '')
  const [database, setDatabase] = useState(initial?.database ?? '')
  const [user, setUser] = useState(initial?.user ?? '')
  const [file, setFile] = useState(initial?.file ?? '')
  const [ssl, setSsl] = useState(initial?.ssl ?? false)
  const [sshHost, setSshHost] = useState(initial?.sshHost ?? '')
  const [password, setPassword] = useState('')
  const effectiveId = id || toId(name)
  const server = kind !== 'sqlite'

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const input: DbConnectionInput = {
      id: effectiveId,
      name: name.trim(),
      kind,
      ...(server
        ? {
            host: host.trim() || '127.0.0.1',
            ...(Number(port) ? { port: Number(port) } : {}),
            ...(database.trim() ? { database: database.trim() } : {}),
            ...(user.trim() ? { user: user.trim() } : {}),
            ...(ssl ? { ssl: true } : {}),
            ...(sshHost ? { sshHost } : {}),
            ...(password ? { password } : {}),
          }
        : { file: file.trim(), clearSecret: true }),
    }
    api.connections
      .saveDb(input)
      .then((view) => {
        setPassword('')
        if (!editing) {
          setName('')
          setId('')
        }
        onSaved(view)
      })
      .catch((err: unknown) => onError(errorMessage(err)))
  }

  return (
    <form className="g9-form" onSubmit={submit} autoComplete="off">
      <label htmlFor={`${fid}-n`}>Ad</label>
      <input id={`${fid}-n`} className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Üretim veritabanı" maxLength={80} required />
      <label htmlFor={`${fid}-i`}>Kimlik</label>
      <input id={`${fid}-i`} className="text-input" value={effectiveId} onChange={(e) => setId(toId(e.target.value))} placeholder={ID_HINT} disabled={editing} spellCheck={false} required />
      <label htmlFor={`${fid}-k`}>Tür</label>
      <select id={`${fid}-k`} className="g7-select" value={kind} onChange={(e) => setKind(e.target.value as DbConnectionInput['kind'])}>
        <option value="postgres">PostgreSQL</option>
        <option value="mysql">MySQL / MariaDB</option>
        <option value="sqlite">SQLite (proje dosyası)</option>
      </select>
      {server ? (
        <>
          <label htmlFor={`${fid}-t`}>SSH tüneli</label>
          <select id={`${fid}-t`} className="g7-select" value={sshHost} onChange={(e) => setSshHost(e.target.value)}>
            <option value="">Yok (doğrudan bağlan)</option>
            {sshHosts.map((h) => (
              <option key={h.id} value={h.id}>
                {h.name} ({h.id})
              </option>
            ))}
          </select>
          <label htmlFor={`${fid}-h`}>{sshHost ? 'Adres (sunucuya göre)' : 'Adres'}</label>
          <input id={`${fid}-h`} className="text-input" value={host} onChange={(e) => setHost(e.target.value)} spellCheck={false} />
          <label htmlFor={`${fid}-p`}>Port</label>
          <input id={`${fid}-p`} className="text-input" type="number" min={1} max={65535} value={port} onChange={(e) => setPort(e.target.value)} placeholder={kind === 'postgres' ? '5432' : '3306'} />
          <label htmlFor={`${fid}-d`}>Veritabanı</label>
          <input id={`${fid}-d`} className="text-input" value={database} onChange={(e) => setDatabase(e.target.value)} spellCheck={false} />
          <label htmlFor={`${fid}-u`}>Kullanıcı</label>
          <input id={`${fid}-u`} className="text-input" value={user} onChange={(e) => setUser(e.target.value)} placeholder="salt okunur kullanıcı önerilir" spellCheck={false} />
          {canStoreSecrets && (
            <>
              <label htmlFor={`${fid}-s`}>Parola</label>
              <input id={`${fid}-s`} className="text-input" type="password" value={password} onChange={(e) => setPassword(e.target.value)} placeholder={initial?.hasSecret ? 'Kayıtlı · değiştirmek için yaz' : ''} autoComplete="new-password" />
            </>
          )}
          <label className="g9-check">
            <input type="checkbox" checked={ssl} onChange={(e) => setSsl(e.target.checked)} /> TLS (sertifika doğrulanır)
          </label>
        </>
      ) : (
        <>
          <label htmlFor={`${fid}-f`}>Dosya (proje klasörüne göre)</label>
          <input id={`${fid}-f`} className="text-input" value={file} onChange={(e) => setFile(e.target.value)} placeholder="data/app.db" spellCheck={false} required />
        </>
      )}
      <div className="g7-button-row">
        <button type="submit" className="btn small" disabled={!effectiveId || !name.trim() || (!server && !file.trim())}>
          {editing ? 'Kaydet' : (<><Plus size={14} aria-hidden="true" /> Veritabanı ekle</>)}
        </button>
      </div>
    </form>
  )
}

function ConnectionCard({
  title,
  subtitle,
  badges,
  onTest,
  onRemove,
  editor,
  result,
  onTrust,
}: {
  title: string
  subtitle: string
  badges: ReactNode
  onTest: () => Promise<void>
  onRemove: () => void
  editor: ReactNode
  result: ConnectionTestResult | null
  onTrust?: (fingerprint: string) => void
}) {
  const [editing, setEditing] = useState(false)
  const [testing, setTesting] = useState(false)
  const titleId = useId()
  const test = () => {
    setTesting(true)
    void onTest().finally(() => setTesting(false))
  }
  return (
    <section className="g7-subsection g9-provider" aria-labelledby={titleId}>
      <h4 id={titleId} className="g7-h4">
        <span>{title}</span>
        {badges}
        <span className="g9-grow" />
        <button type="button" className="btn small" onClick={test} disabled={testing}>
          {testing ? 'Test ediliyor...' : 'Bağlantıyı test et'}
        </button>
        <button type="button" className="action-btn" onClick={() => setEditing((v) => !v)} aria-label={`${title} düzenle`} aria-expanded={editing} title="Düzenle">
          <Pencil size={14} aria-hidden="true" />
        </button>
        <button type="button" className="action-btn" onClick={onRemove} aria-label={`${title} sil`} title="Sil">
          <Trash2 size={14} aria-hidden="true" />
        </button>
      </h4>
      <p className="g7-mono g9-url">{subtitle}</p>
      <TestResult result={result} {...(onTrust ? { onTrust } : {})} />
      {editing && editor}
    </section>
  )
}

export function ConnectionsSection() {
  const { data, error, mutate } = useSWR(KEY, () => api.connections.list(), { revalidateOnFocus: false })
  const [message, setMessage] = useState<string | null>(null)
  const [results, setResults] = useState<Record<string, ConnectionTestResult>>({})
  const setView: Setter = (view) => {
    setMessage(null)
    void mutate(view, { revalidate: false })
  }
  const setResult = (id: string, r: ConnectionTestResult) => setResults((prev) => ({ ...prev, [id]: r }))
  const clearResult = (id: string) =>
    setResults((prev) => {
      const { [id]: _removed, ...rest } = prev
      return rest
    })

  if (error) return <p className="error-text">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>

  const remove = async (id: string, title: string) => {
    if (!(await confirmAction({ title: `${title} silinsin mi?`, message: 'Kayıtlı parola ve onaylı sunucu anahtarı da silinir.', confirmLabel: 'Sil', danger: true }))) return
    api.connections.remove(id).then(setView).catch((err: unknown) => setMessage(errorMessage(err)))
  }
  const trust = (id: string) => (fingerprint: string) => {
    api.connections
      .trustHostKey(id, fingerprint)
      .then(async (view) => {
        setView(view)
        setResult(id, await api.connections.testSsh(id))
      })
      .catch((err: unknown) => setMessage(errorMessage(err)))
  }

  return (
    <div className="g7-stack">
      <p className="g7-note text-pretty">
        Ajan burada tanımlı sunuculara SSH ile bağlanıp komut çalıştırabilir, dosya okuyup yazabilir, kurulum yapabilir ve veritabanlarında salt okunur sorgu çalıştırabilir.
        Uzak komutlar ve yazma işlemleri yerel komutlar gibi onayına sunulur. Parolalar {data.canStoreSecrets ? 'işletim sisteminin şifreli kasasında tutulur ve modele gönderilmez' : 'bu sistemde kasa olmadığı için kaydedilemez; anahtar dosyası veya ssh-agent kullan'}.
      </p>
      {message && <p className="error-text" role="alert">{message}</p>}

      <h3 className="g7-h4 g31-heading">
        <Server size={16} aria-hidden="true" /> SSH sunucuları
      </h3>
      {data.ssh.length === 0 && <p className="muted">Henüz sunucu yok.</p>}
      {data.ssh.map((h) => (
        <ConnectionCard
          key={h.id}
          title={h.name}
          subtitle={`${h.id} · ${h.username}@${h.host}:${h.port}`}
          badges={
            <>
              <span className="badge neutral">{h.auth === 'password' ? 'parola' : h.auth === 'key' ? 'anahtar' : 'agent'}</span>
              {h.hostKey ? <span className="badge g9-on">anahtar onaylı</span> : <span className="badge g9-health bad">anahtar onaysız</span>}
            </>
          }
          onTest={async () => {
            clearResult(h.id)
            setResult(h.id, await api.connections.testSsh(h.id))
          }}
          onRemove={() => void remove(h.id, h.name)}
          result={results[h.id] ?? null}
          onTrust={trust(h.id)}
          editor={<SshForm initial={h} canStoreSecrets={data.canStoreSecrets} onSaved={setView} onError={setMessage} />}
        />
      ))}
      <section className="g7-subsection">
        <h4 className="g7-h4">Sunucu ekle</h4>
        <SshForm canStoreSecrets={data.canStoreSecrets} onSaved={setView} onError={setMessage} />
      </section>

      <h3 className="g7-h4 g31-heading">
        <Database size={16} aria-hidden="true" /> Veritabanları
      </h3>
      {data.databases.length === 0 && <p className="muted">Henüz veritabanı yok.</p>}
      {data.databases.map((d) => (
        <ConnectionCard
          key={d.id}
          title={d.name}
          subtitle={d.kind === 'sqlite' ? `${d.id} · sqlite · ${d.file ?? ''}` : `${d.id} · ${d.kind} · ${d.user ?? ''}@${d.host ?? '127.0.0.1'}:${d.port ?? ''}/${d.database ?? ''}`}
          badges={<>{d.sshHost && <span className="badge neutral">tünel: {d.sshHost}</span>}</>}
          onTest={async () => {
            clearResult(d.id)
            setResult(d.id, await api.connections.testDb(d.id))
          }}
          onRemove={() => void remove(d.id, d.name)}
          result={results[d.id] ?? null}
          editor={<DbForm initial={d} sshHosts={data.ssh} canStoreSecrets={data.canStoreSecrets} onSaved={setView} onError={setMessage} />}
        />
      ))}
      <section className="g7-subsection">
        <h4 className="g7-h4">Veritabanı ekle</h4>
        <DbForm sshHosts={data.ssh} canStoreSecrets={data.canStoreSecrets} onSaved={setView} onError={setMessage} />
      </section>
    </div>
  )
}
