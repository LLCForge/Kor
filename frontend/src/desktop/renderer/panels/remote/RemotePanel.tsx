// Uzak sunucular görünümü (§15.3, §15.16, ADR 0044): tanımlı/taranmış hostlar, bağlantı durumu, uzak görev
// dökümü (onay kartlarıyla) ve keşif raporu (servisler, portlar, sanal hostlar). Arka uç `remote.*`.
import { CircleAlert, Plug, PlugZap, Radar, RefreshCw, Send, Square } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import type { Discovery, InfraSnapshot, RemoteConnected } from '../../../ipc-remote.ts'
import { errorMessage } from '../../api.ts'
import type { KorPanelProps } from '../registry.ts'
import { remoteApi } from './bridge.ts'
import { EMPTY, memText, pendingConfirm, reduce, type Session, taskStateText } from './shape.ts'

function Hosts({ selected, onSelect }: { selected: string; onSelect: (t: string) => void }) {
  const id = useId()
  const { data, error, isLoading, mutate } = useSWR('remote:hosts', () => remoteApi.hosts())
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        Sunucular
        <button type="button" className="btn small" onClick={() => void mutate()} aria-label="Listeyi yenile" title="Listeyi yenile" data-kor-cmd="remote.hosts">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </h3>
      {isLoading && <p className="muted" role="status">Yükleniyor…</p>}
      {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
      {data && !data.length && (
        <p className="ks-empty">
          Tanımlı sunucu yok. connections.toml dosyasına <code translate="no">[ssh.ad]</code> ekleyin ya da aşağıya <code translate="no">kullanıcı@sunucu</code> yazın.
        </p>
      )}
      {data && data.length > 0 && (
        <ul className="ks-list">
          {data.map((h) => (
            <li key={h.id} className="ks-row">
              <label className="ks-check">
                <input type="radio" name={`${id}-h`} checked={selected === h.id} onChange={() => onSelect(h.id)} />
                <span translate="no">{h.id}</span>
                <span className="muted" translate="no">{h.address}</span>
                <span className="muted">{h.scannedAt ? `taranma: ${new Date(h.scannedAt * 1000).toLocaleString()}` : 'taranmadı'}</span>
              </label>
            </li>
          ))}
        </ul>
      )}
      <label className="ks-field">
        Adres
        <input className="text-input" value={selected} onChange={(e) => onSelect(e.target.value.trim())} placeholder="kullanıcı@sunucu:22" maxLength={300} spellCheck={false} translate="no" />
      </label>
    </section>
  )
}

function Connection({ target, conn, setConn }: { target: string; conn: RemoteConnected | null; setConn: (c: RemoteConnected | null) => void }) {
  const id = useId()
  const [project, setProject] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [session, setSession] = useState<Session>(EMPTY)
  const [text, setText] = useState('')

  useEffect(
    () =>
      remoteApi.onEvent((e) => {
        if (e.target === target) setSession((s) => reduce(s, e.event))
      }),
    [target],
  )
  useEffect(
    () =>
      remoteApi.onClosed((e) => {
        if (e.target === target) setConn(null)
      }),
    [target, setConn],
  )
  useEffect(() => setSession(EMPTY), [target])

  const run = async (f: () => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    try {
      await f()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const connect = () =>
    run(async () => {
      const c = await remoteApi.connect(target, project || undefined)
      setConn(c)
      setSession({ ...EMPTY, tasks: c.tasks })
    })

  const confirm = pendingConfirm(session)
  const live = conn?.target === target

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`} data-kor-event="kor:remote/event kor:remote/closed">
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        Uzak oturum
        <span className={`ks-badge${live ? ' on' : ''}`}>{live ? 'Bağlı' : 'Bağlı değil'}</span>
      </h3>
      {!live ? (
        <form className="ks-form" onSubmit={(e) => { e.preventDefault(); void connect() }}>
          <label className="ks-field">
            Uzak proje klasörü
            <input className="text-input" value={project} onChange={(e) => setProject(e.target.value)} placeholder="/srv/uygulama (boş: ev klasörü)" maxLength={1024} spellCheck={false} translate="no" />
          </label>
          <p className="muted">Ajan, indeks, graf ve sandbox sunucuda çalışır. İlk bağlantıda imzası doğrulanmış Kor sunucuya kurulur; sonra sürüm eşlenir. Yalnızca SSH anahtarınız/ajanınız kullanılır.</p>
          <button type="submit" className="btn small primary" disabled={busy || !target} data-kor-cmd="remote.connect">
            <Plug size={14} aria-hidden="true" />
            {busy ? 'Bağlanıyor…' : 'Bağlan'}
          </button>
        </form>
      ) : (
        <>
          <dl className="ks-facts">
            <dt>Sistem</dt>
            <dd translate="no">{conn.os} {conn.arch}</dd>
            <dt>Kor</dt>
            <dd translate="no">{conn.version ?? '—'}{conn.replaced === '' ? ' (yeni kuruldu)' : conn.replaced ? ` (${conn.replaced} sürümünden güncellendi)` : ''}</dd>
            <dt>Proje</dt>
            <dd translate="no">{conn.project}</dd>
            <dt>Görev</dt>
            <dd translate="no">{session.task ?? '—'}{session.running ? ' · çalışıyor' : ''}</dd>
          </dl>
          {session.tasks.length > 0 && (
            <ul className="ks-list" aria-label="Uzak görevler">
              {session.tasks.map((t) => (
                <li key={t.id} className="ks-row">
                  <span translate="no">{t.prompt.split('\n')[0]}</span>
                  <span className="muted">{taskStateText(t.state)}</span>
                  <button type="button" className="btn small" onClick={() => void run(() => remoteApi.attach(target, t.id))} data-kor-cmd="remote.attach">İzle</button>
                </li>
              ))}
            </ul>
          )}
          <div className="kr-log" aria-live="polite">
            {session.rows.length === 0 && <p className="ks-empty">Henüz olay yok. Bir görev yazın ya da listeden birini izleyin.</p>}
            {session.rows.map((r, i) => {
              switch (r.kind) {
                case 'text':
                  return <p key={i} className="kr-text" translate="no">{r.text}</p>
                case 'tool':
                  return <p key={i} className="kr-tool" translate="no"><code>{r.name}</code> {r.summary}{r.result ? ` → ${r.result.split('\n')[0]}` : ''}</p>
                case 'confirm':
                  return <p key={i} className={`kr-confirm${r.danger ? ' danger' : ''}`}><code translate="no">{r.name}</code> <span translate="no">{r.summary}</span> {r.answer ? `· yanıt: ${r.answer}` : '· onay bekliyor'}</p>
                case 'error':
                  return <p key={i} className="error-text" role="alert">{r.text}</p>
                default:
                  return <p key={i} className="muted" translate="no">{r.text}</p>
              }
            })}
          </div>
          {confirm && (
            <div className="ks-actions" role="group" aria-label="Onay">
              <button type="button" className={`btn small ${confirm.danger ? 'danger' : 'primary'}`} onClick={() => void run(() => remoteApi.approve(target, confirm.id, 'yes'))} data-kor-cmd="remote.approve">İzin ver</button>
              {!confirm.danger && <button type="button" className="btn small" onClick={() => void run(() => remoteApi.approve(target, confirm.id, 'always'))} data-kor-cmd="remote.approve">Her zaman</button>}
              <button type="button" className="btn small" onClick={() => void run(() => remoteApi.approve(target, confirm.id, 'no'))} data-kor-cmd="remote.approve">Reddet</button>
            </div>
          )}
          <form
            className="ks-inline"
            onSubmit={(e) => {
              e.preventDefault()
              const t = text.trim()
              if (!t) return
              setText('')
              void run(() => remoteApi.send(target, t, session.task ?? undefined))
            }}
          >
            <input className="text-input" value={text} onChange={(e) => setText(e.target.value)} placeholder="Sunucudaki ajana görev yazın" aria-label="Uzak görev" maxLength={20000} />
            <button type="submit" className="btn small primary" disabled={busy || !text.trim()} aria-label="Gönder" title="Gönder" data-kor-cmd="remote.send">
              <Send size={14} aria-hidden="true" />
            </button>
            <button type="button" className="btn small" disabled={!session.running} onClick={() => void run(() => remoteApi.abort(target))} aria-label="Turu durdur" title="Turu durdur" data-kor-cmd="remote.abort">
              <Square size={14} aria-hidden="true" />
            </button>
          </form>
          <div className="ks-actions">
            <button type="button" className="btn small" onClick={() => void run(() => remoteApi.tasks(target))} data-kor-cmd="remote.tasks">Görevleri yenile</button>
            <button type="button" className="btn small" onClick={() => void run(async () => { await remoteApi.disconnect(target); setConn(null) })} data-kor-cmd="remote.disconnect">
              <PlugZap size={14} aria-hidden="true" />
              Ayrıl (görev sürer)
            </button>
          </div>
        </>
      )}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function Report({ snap }: { snap: InfraSnapshot }) {
  return (
    <>
      <dl className="ks-facts">
        <dt>Sistem</dt>
        <dd translate="no">{snap.os} · {snap.kernel} · {snap.arch}</dd>
        <dt>Bellek</dt>
        <dd>{memText(snap.memory)}</dd>
        <dt>Diskler</dt>
        <dd translate="no">{snap.disks.map((d) => `${d.mount} ${d.use_pct}`).join(', ') || '—'}</dd>
        <dt>Diller</dt>
        <dd translate="no">{snap.languages.map((l) => `${l.name} ${l.version}`).join(', ') || '—'}</dd>
      </dl>
      <h4>Servisler ({snap.services.length})</h4>
      {snap.services.length === 0 ? (
        <p className="ks-empty">Servis bulunamadı.</p>
      ) : (
        <table className="kr-table">
          <thead>
            <tr><th>Tür</th><th>Ad</th><th>Durum</th><th>Port</th><th>Alan adı</th></tr>
          </thead>
          <tbody translate="no">
            {snap.services.map((s) => (
              <tr key={`${s.kind}/${s.name}`} title={s.detail}>
                <td>{s.kind}</td><td>{s.name}</td><td>{s.state}</td><td>{s.ports.join(', ')}</td><td>{s.domains.join(', ')}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <h4>Açık portlar ({snap.ports.length})</h4>
      {snap.ports.length === 0 ? (
        <p className="ks-empty">Dinleyen TCP portu bulunamadı.</p>
      ) : (
        <table className="kr-table">
          <thead>
            <tr><th>Port</th><th>Adres</th><th>Süreç</th></tr>
          </thead>
          <tbody translate="no">
            {snap.ports.map((p) => (
              <tr key={`${p.address}:${p.port}`}><td>{p.port}</td><td>{p.address}</td><td>{p.process || '—'}</td></tr>
            ))}
          </tbody>
        </table>
      )}
      <h4>Sanal hostlar ({snap.vhosts.length})</h4>
      {snap.vhosts.length === 0 ? (
        <p className="ks-empty">nginx/Caddy sanal hostu bulunamadı.</p>
      ) : (
        <ul className="ks-list" translate="no">
          {snap.vhosts.map((v, i) => (
            <li key={i} className="ks-row">{v.server}: {v.names.join(', ')} → {v.upstreams.join(', ')}{v.services.length ? ` (${v.services.join(', ')})` : ''}</li>
          ))}
        </ul>
      )}
      {snap.schedules.length > 0 && (
        <>
          <h4>Cron ve zamanlayıcılar ({snap.schedules.length})</h4>
          <ul className="ks-list" translate="no">
            {snap.schedules.map((c, i) => <li key={i} className="ks-row">{c.when}: {c.what}</li>)}
          </ul>
        </>
      )}
      {snap.env_files.length > 0 && (
        <>
          <h4>Env dosyaları (yalnızca yollar)</h4>
          <ul className="ks-list" translate="no">
            {snap.env_files.map((f) => <li key={f} className="ks-row"><code>{f}</code></li>)}
          </ul>
        </>
      )}
      {snap.notes.length > 0 && (
        <ul className="ks-list">
          {snap.notes.map((n, i) => <li key={i} className="ks-warn" translate="no">{n}</li>)}
        </ul>
      )}
    </>
  )
}

function Discover({ target }: { target: string }) {
  const id = useId()
  const host = target.split('@').pop()?.replace(/:\d+$/, '') ?? ''
  const stored = useSWR(host ? ['remote:report', host] : null, () => remoteApi.report(host))
  const [result, setResult] = useState<Discovery | null>(null)
  const [sudo, setSudo] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => { setResult(null); setSudo([]) }, [target])

  const scan = async () => {
    setBusy(true)
    setError(null)
    try {
      const d = await remoteApi.discover(target, sudo)
      setResult(d)
      void stored.mutate(d.snapshot, { revalidate: false })
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  const snap = result?.snapshot ?? stored.data ?? null

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <Radar size={16} aria-hidden="true" />
        Sistem keşfi
      </h3>
      <p className="muted">Salt okunur tarama: hiçbir şey değiştirilmez, env dosyalarının yalnızca yolları okunur. Sonuç Kor hafızasına yazılır ve açık projenin grafına bağlanır.</p>
      {result && result.sudoOffered.length > 0 && (
        <fieldset className="ko-models">
          <legend>Yetki gereken adımlar: sudo ile yeniden çalıştırmak istediklerinizi tek tek seçin</legend>
          {result.sudoOffered.map((o) => (
            <label key={o.step} className="ks-check">
              <input type="checkbox" checked={sudo.includes(o.step)} onChange={(e) => setSudo(e.target.checked ? [...sudo, o.step] : sudo.filter((s) => s !== o.step))} />
              <span translate="no">{o.step}: {o.reason} — <code>{o.command}</code></span>
            </label>
          ))}
        </fieldset>
      )}
      <button type="button" className="btn small primary" disabled={busy || !target} onClick={() => void scan()} data-kor-cmd="remote.discover remote.report">
        <Radar size={14} aria-hidden="true" />
        {busy ? 'Taranıyor…' : sudo.length ? 'Seçili adımlarla yeniden tara' : 'Tara'}
      </button>
      {result && <p className="ks-ok" translate="no">{result.markdown}{result.linked ? ' · projeye bağlandı' : ''}</p>}
      {error && <p className="error-text" role="alert"><CircleAlert size={14} aria-hidden="true" />{error}</p>}
      {stored.isLoading && <p className="muted" role="status">Yükleniyor…</p>}
      {snap ? <Report snap={snap} /> : !stored.isLoading && <p className="ks-empty">Bu sunucu henüz taranmadı.</p>}
    </section>
  )
}

export function RemotePanel(_props: KorPanelProps) {
  const [target, setTarget] = useState('')
  const [conn, setConn] = useState<RemoteConnected | null>(null)
  return (
    <div className="ks kr">
      <Hosts selected={target} onSelect={setTarget} />
      {target ? (
        <>
          <Connection target={target} conn={conn} setConn={setConn} />
          <Discover target={target} />
        </>
      ) : (
        <p className="ks-empty">Bir sunucu seçin ya da adres yazın.</p>
      )}
    </div>
  )
}
