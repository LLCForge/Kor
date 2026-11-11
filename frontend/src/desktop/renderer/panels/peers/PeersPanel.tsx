// Makineler görünümü (§31.1, §28.8, ADR 0057): bu makine ve eşleştirme (kod + QR), eşleşmiş makineler
// (yük, en boş, yetki matrisi, yol, hafıza eşitleme), görev devretme (onaylar burada, sonuç fark olarak
// döner), laptop kapanınca devretme ve editörler için ACP. Arka uç `peer.*`, `acp.*`.
import { ArrowRightLeft, KeyRound, Link2, Play, QrCode, RefreshCw, Server, Square, Trash2 } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import type { AcpPermission, PeerApplied, PeerPairCode, PeerResult, PeerRow } from '../../../ipc.gen.ts'
import type { RemoteWireEvent } from '../../../ipc-remote.ts'
import { errorMessage } from '../../api.ts'
import { CopyButton } from '../../CopyButton.tsx'
import type { KorPanelProps } from '../registry.ts'
import { EMPTY, pendingConfirm, reduce, type Session } from '../remote/shape.ts'
import { acpApi, peerApi } from './bridge.ts'
import { fromForm, grantText, loadText, qrCells, sortPeers, toForm, type GrantForm } from './shape.ts'

const LIST_KEY = 'peer:list'

/** Bir işlemi meşgul/hata durumuyla çalıştırır. */
function useAction() {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
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
  return { busy, error, run }
}

function Qr({ code }: { code: PeerPairCode }) {
  const cells = qrCells(code.size, code.modules)
  return (
    <svg className="kpe-qr" viewBox={`-2 -2 ${code.size + 4} ${code.size + 4}`} role="img" aria-label="Eşleştirme kodu QR">
      {cells.map(([x, y]) => (
        <rect key={`${x}-${y}`} x={x} y={y} width={1} height={1} />
      ))}
    </svg>
  )
}

function ThisMachine() {
  const id = useId()
  const list = useSWR(LIST_KEY, () => peerApi.list())
  const { busy, error, run } = useAction()
  const [code, setCode] = useState<PeerPairCode | null>(null)
  const [join, setJoin] = useState('')
  const [ssh, setSsh] = useState('')
  const [paired, setPaired] = useState<string | null>(null)

  useEffect(
    () =>
      peerApi.onPaired((e) => {
        setPaired(e.name)
        setCode(null)
        void list.mutate()
      }),
    [list],
  )

  const me = list.data?.self
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`} data-kor-event="kor:peer/paired">
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <Server size={16} aria-hidden="true" />
        Bu makine
        <span className={`ks-badge${list.data?.serving ? ' on' : ''}`}>{list.data?.serving ? `Görev kabul ediyor (${list.data.serving})` : 'Görev kabul etmiyor'}</span>
      </h3>
      {list.isLoading && <p className="muted" role="status">Yükleniyor…</p>}
      {list.error && <p className="error-text" role="alert">{errorMessage(list.error)}</p>}
      {me && (
        <dl className="ks-facts">
          <dt>Ad</dt>
          <dd translate="no">{me.name}</dd>
          <dt>Anahtar</dt>
          <dd translate="no">{me.fingerprint}</dd>
          <dt>Dinleme adresi</dt>
          <dd translate="no">{me.listen}</dd>
        </dl>
      )}
      <div className="ks-actions">
        <button type="button" className="btn small" disabled={busy} onClick={() => void run(async () => { await peerApi.serve(!list.data?.serving); await list.mutate() })} data-kor-cmd="peer.serve">
          {list.data?.serving ? 'Görev kabulünü durdur' : 'Görev kabul et'}
        </button>
        <button type="button" className="btn small primary" disabled={busy} onClick={() => void run(async () => { setPaired(null); setCode(await peerApi.pairCode()) })} data-kor-cmd="peer.pairCode">
          <QrCode size={14} aria-hidden="true" />
          Eşleştirme kodu üret
        </button>
      </div>
      {code && (
        <div className="kpe-pair">
          <Qr code={code} />
          <div className="ks-form">
            <p className="muted">Diğer makinede çalıştırın ya da QR'ı okutun. Kod saat {new Date(code.expiresAt).toLocaleTimeString()} kadar ve tek denemede geçerli.</p>
            <code className="kpe-code" translate="no">kor peer pair --join {code.code}</code>
            <CopyButton getText={() => `kor peer pair --join ${code.code}`} label="Komutu kopyala" />
            <p className="muted">Eşleşince iki ekrandaki anahtarı karşılaştırın: <span translate="no">{code.fingerprint}</span></p>
          </div>
        </div>
      )}
      {paired && <p className="ks-ok" role="status"><span translate="no">{paired}</span> ile eşleşildi.</p>}
      <form className="ks-form" onSubmit={(e) => { e.preventDefault(); void run(async () => { const r = await peerApi.join(join.trim(), ssh.trim() || undefined); setPaired(r.name); setJoin(''); await list.mutate() }) }}>
        <label className="ks-field">
          Başka makinenin kodu
          <input className="text-input" value={join} onChange={(e) => setJoin(e.target.value)} placeholder="KOR1-XXXX-XXXX-XXXX-XXXX-XXXX@192.168.1.5:47321" maxLength={600} spellCheck={false} translate="no" />
        </label>
        <label className="ks-field">
          SSH ile ulaş (isteğe bağlı)
          <input className="text-input" value={ssh} onChange={(e) => setSsh(e.target.value)} placeholder="kullanıcı@sunucu" maxLength={300} spellCheck={false} translate="no" />
        </label>
        <button type="submit" className="btn small" disabled={busy || !join.trim()} data-kor-cmd="peer.join">
          <Link2 size={14} aria-hidden="true" />
          Eşleş
        </button>
      </form>
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function GrantEditor({ peer, onDone }: { peer: PeerRow; onDone: () => void }) {
  const [f, setF] = useState<GrantForm>(() => toForm(peer.grant))
  const { busy, error, run } = useAction()
  return (
    <form className="ks-form" onSubmit={(e) => { e.preventDefault(); void run(async () => { await peerApi.allow(peer.name, fromForm(f)); onDone() }) }}>
      <p className="muted">Varsayılan salt-okunur: dizin, yazma, komut ve araç ancak burada verilirse kullanılabilir. Görevin kendi proje kopyası her zaman yazılabilir.</p>
      <div className="kpe-grid">
        <label className="ks-field">
          Dizinler (satır başına bir)
          <textarea className="text-input" value={f.dirs} onChange={(e) => setF({ ...f, dirs: e.target.value })} spellCheck={false} translate="no" />
        </label>
        <label className="ks-field">
          Komut önekleri (* = hepsi)
          <textarea className="text-input" value={f.commands} onChange={(e) => setF({ ...f, commands: e.target.value })} spellCheck={false} translate="no" />
        </label>
        <label className="ks-field">
          Ek araçlar (WebFetch, network, mcp__…)
          <textarea className="text-input" value={f.tools} onChange={(e) => setF({ ...f, tools: e.target.value })} spellCheck={false} translate="no" />
        </label>
        <div className="ks-form">
          <label className="ks-check">
            <input type="checkbox" checked={f.write} onChange={(e) => setF({ ...f, write: e.target.checked })} />
            Bu dizinlerde dosya değiştirebilir
          </label>
          <label className="ks-check">
            <input type="checkbox" checked={f.memory} onChange={(e) => setF({ ...f, memory: e.target.checked })} />
            Kayıtları bu makinenin hafızasına birleştirilir
          </label>
        </div>
      </div>
      <button type="submit" className="btn small primary" disabled={busy} data-kor-cmd="peer.allow">Yetkileri kaydet</button>
      {error && <p className="error-text" role="alert">{error}</p>}
    </form>
  )
}

function RouteEditor({ peer, onDone }: { peer: PeerRow; onDone: () => void }) {
  const [ssh, setSsh] = useState(peer.ssh ?? '')
  const [addrs, setAddrs] = useState(peer.addrs.join('\n'))
  const { busy, error, run } = useAction()
  return (
    <form className="ks-form" onSubmit={(e) => { e.preventDefault(); void run(async () => { await peerApi.route(peer.name, ssh.trim() || null, addrs.split(/\r?\n/).map((a) => a.trim()).filter(Boolean)); onDone() }) }}>
      <p className="muted">Sıra: Tailscale adresi (ts:), doğrudan adres, sonra SSH. Her yolda aynı anahtar doğrulanır.</p>
      <div className="kpe-grid">
        <label className="ks-field">
          Adresler (satır başına bir)
          <textarea className="text-input" value={addrs} onChange={(e) => setAddrs(e.target.value)} spellCheck={false} translate="no" />
        </label>
        <label className="ks-field">
          SSH hedefi
          <input className="text-input" value={ssh} onChange={(e) => setSsh(e.target.value)} placeholder="kullanıcı@sunucu" maxLength={300} spellCheck={false} translate="no" />
        </label>
      </div>
      <button type="submit" className="btn small primary" disabled={busy} data-kor-cmd="peer.route">Yolu kaydet</button>
      {error && <p className="error-text" role="alert">{error}</p>}
    </form>
  )
}

function Peers({ onTasks }: { onTasks: (peer: string, task: string) => void }) {
  const id = useId()
  const list = useSWR(LIST_KEY, () => peerApi.list())
  const { busy, error, run } = useAction()
  const [open, setOpen] = useState<{ name: string; what: 'grant' | 'route' | 'tasks' | 'remove' } | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const tasks = useSWR(open?.what === 'tasks' ? ['peer:tasks', open.name] : null, () => peerApi.tasks(open!.name))
  const rows = sortPeers(list.data?.peers ?? [])
  const done = () => {
    setOpen(null)
    void list.mutate()
  }
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        Eşleşmiş makineler
        <button type="button" className="btn small" onClick={() => void list.mutate()} aria-label="Yükleri yenile" title="Yükleri yenile" data-kor-cmd="peer.list">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </h3>
      {list.data && rows.length === 0 && <p className="ks-empty">Eşleşmiş makine yok. Yukarıdan kod üretin ya da başka makinenin koduyla eşleşin.</p>}
      <ul className="ks-list">
        {rows.map((p) => (
          <li key={p.name} className="kpe-row">
            <div className="kpe-row-head">
              <strong translate="no">{p.name}</strong>
              {list.data?.leastBusy === p.name && <span className="ks-badge on">En boş</span>}
              <span className="muted">{p.error ? `ulaşılamıyor: ${p.error}` : loadText(p.load)}</span>
            </div>
            <span className="muted">Yetki: {grantText(p.grant)} · anahtar <span translate="no">{p.fingerprint}</span></span>
            <div className="ks-actions">
              <button type="button" className="btn small" onClick={() => setOpen({ name: p.name, what: 'grant' })} data-kor-cmd="peer.allow">
                <KeyRound size={14} aria-hidden="true" />
                Yetkiler
              </button>
              <button type="button" className="btn small" onClick={() => setOpen({ name: p.name, what: 'route' })} data-kor-cmd="peer.route">Yol</button>
              <button type="button" className="btn small" onClick={() => setOpen({ name: p.name, what: 'tasks' })} data-kor-cmd="peer.tasks">Görevler</button>
              <button
                type="button"
                className="btn small"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    const r = await peerApi.sync(p.name)
                    setNote(`${p.name}: ${r.records} kayıt; burada ${r.local.added} eklendi, ${r.local.updated} güncellendi, ${r.local.removed} silindi; ${r.conflicts.length} çakışma${r.remoteApplied ? '' : ' (karşı taraf salt-okunur)'}`)
                  })
                }
                data-kor-cmd="peer.sync"
              >
                <ArrowRightLeft size={14} aria-hidden="true" />
                Hafızayı eşitle
              </button>
              <button type="button" className="btn small danger" onClick={() => setOpen({ name: p.name, what: 'remove' })} aria-label={`${p.name}: unut`} title="Unut" data-kor-cmd="peer.remove">
                <Trash2 size={14} aria-hidden="true" />
              </button>
            </div>
            {open?.name === p.name && open.what === 'grant' && <GrantEditor peer={p} onDone={done} />}
            {open?.name === p.name && open.what === 'route' && <RouteEditor peer={p} onDone={done} />}
            {open?.name === p.name && open.what === 'remove' && (
              <div className="ks-actions" role="group" aria-label="Unutmayı onayla">
                <span className="ks-warn">Bu makine unutulsun mu? Yeniden eşleştirmek gerekir.</span>
                <button type="button" className="btn small danger" onClick={() => void run(async () => { await peerApi.remove(p.name); done() })} data-kor-cmd="peer.remove">Unut</button>
                <button type="button" className="btn small" onClick={() => setOpen(null)}>Vazgeç</button>
              </div>
            )}
            {open?.name === p.name && open.what === 'tasks' && (
              <>
                {tasks.isLoading && <p className="muted" role="status">Yükleniyor…</p>}
                {tasks.error && <p className="error-text" role="alert">{errorMessage(tasks.error)}</p>}
                {tasks.data && tasks.data.length === 0 && <p className="ks-empty">Bu makinede görev yok.</p>}
                {tasks.data?.map((t) => (
                  <div key={t.id} className="ks-row">
                    <span translate="no">{t.prompt.split('\n')[0]}</span>
                    <span className="muted">{t.state}</span>
                    <button type="button" className="btn small" onClick={() => onTasks(p.name, t.id)} data-kor-cmd="peer.attach">İzle</button>
                  </div>
                ))}
              </>
            )}
          </li>
        ))}
      </ul>
      {note && <p className="ks-ok" role="status" translate="no">{note}</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function Delegate({ follow, setFollow }: { follow: { peer: string; task: string } | null; setFollow: (f: { peer: string; task: string } | null) => void }) {
  const id = useId()
  const list = useSWR(LIST_KEY, () => peerApi.list())
  const [target, setTarget] = useState('auto')
  const [text, setText] = useState('')
  const [dir, setDir] = useState('')
  const [session, setSession] = useState<Session>(EMPTY)
  const [result, setResult] = useState<PeerResult | null>(null)
  const [pick, setPick] = useState<string[]>([])
  const [applied, setApplied] = useState<PeerApplied[] | null>(null)
  const { busy, error, run } = useAction()

  useEffect(
    () =>
      peerApi.onEvent((e) => {
        if (!follow || e.peer !== follow.peer || (e.task && e.task !== follow.task)) return
        setSession((s) => reduce(s, e.event as RemoteWireEvent))
        if (e.event.type === 'done') {
          void peerApi.result(follow.peer, follow.task).then((r) => {
            setResult(r)
            setPick(r.files.map((f) => f.path))
          }, () => undefined)
        }
      }),
    [follow],
  )

  const start = () =>
    run(async () => {
      setSession(EMPTY)
      setResult(null)
      setApplied(null)
      const r = await peerApi.run(target, text.trim(), dir.trim() || undefined)
      setFollow(r)
      setText('')
    })
  const confirm = pendingConfirm(session)
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`} data-kor-event="kor:peer/event">
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        Görev devret
        {follow && <span className={`ks-badge${session.running ? ' on' : ''}`} translate="no">{follow.peer} · {follow.task}</span>}
      </h3>
      <form className="ks-form" onSubmit={(e) => { e.preventDefault(); if (text.trim()) void start() }}>
        <div className="kpe-grid">
          <label className="ks-field">
            Makine
            <select className="text-input" value={target} onChange={(e) => setTarget(e.target.value)}>
              <option value="auto">En boş makine{list.data?.leastBusy ? ` (${list.data.leastBusy})` : ''}</option>
              {list.data?.peers.map((p) => (
                <option key={p.name} value={p.name}>{p.name}</option>
              ))}
            </select>
          </label>
          <label className="ks-field">
            Karşı makinedeki dizin (boş: bu projenin kopyası)
            <input className="text-input" value={dir} onChange={(e) => setDir(e.target.value)} placeholder="/srv/uygulama" maxLength={1024} spellCheck={false} translate="no" />
          </label>
        </div>
        <label className="ks-field">
          Görev
          <textarea className="text-input" value={text} onChange={(e) => setText(e.target.value)} placeholder="Testleri sunucuda çalıştır ve kırılanları düzelt" maxLength={20000} />
        </label>
        <button type="submit" className="btn small primary" disabled={busy || !text.trim() || !list.data?.peers.length} data-kor-cmd="peer.run">
          <Play size={14} aria-hidden="true" />
          {busy ? 'Gönderiliyor…' : 'Çalıştır'}
        </button>
      </form>
      {follow && (
        <>
          <div className="kr-log" aria-live="polite">
            {session.rows.length === 0 && <p className="ks-empty">Olaylar bekleniyor…</p>}
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
              <button type="button" className={`btn small ${confirm.danger ? 'danger' : 'primary'}`} onClick={() => void run(() => peerApi.approve(follow.peer, follow.task, confirm.id, 'yes'))} data-kor-cmd="peer.approve">İzin ver</button>
              {!confirm.danger && <button type="button" className="btn small" onClick={() => void run(() => peerApi.approve(follow.peer, follow.task, confirm.id, 'always'))} data-kor-cmd="peer.approve">Her zaman</button>}
              <button type="button" className="btn small" onClick={() => void run(() => peerApi.approve(follow.peer, follow.task, confirm.id, 'no'))} data-kor-cmd="peer.approve">Reddet</button>
            </div>
          )}
          <div className="ks-actions">
            <button type="button" className="btn small" disabled={!session.running} onClick={() => void run(() => peerApi.abort(follow.peer, follow.task))} aria-label="Turu durdur" title="Turu durdur" data-kor-cmd="peer.abort">
              <Square size={14} aria-hidden="true" />
            </button>
            <button type="button" className="btn small" onClick={() => void run(() => peerApi.attach(follow.peer, follow.task))} data-kor-cmd="peer.attach">Yeniden izle</button>
            <button type="button" className="btn small" onClick={() => void run(async () => { const r = await peerApi.result(follow.peer, follow.task); setResult(r); setPick(r.files.map((f) => f.path)) })} data-kor-cmd="peer.result">Sonucu getir</button>
          </div>
        </>
      )}
      {result && (
        result.files.length === 0 ? (
          <p className="ks-empty">Görev bu projede dosya değiştirmedi.</p>
        ) : (
          <>
            <fieldset className="ko-models">
              <legend>Değişen dosyalar: uygulanacakları seçin</legend>
              {result.files.map((f) => (
                <label key={f.path} className="ks-check">
                  <input type="checkbox" checked={pick.includes(f.path)} onChange={(e) => setPick(e.target.checked ? [...pick, f.path] : pick.filter((x) => x !== f.path))} />
                  <span translate="no">{f.path}</span>
                  <span className="muted">{f.kind === 'added' ? 'yeni' : f.kind === 'deleted' ? 'silindi' : 'değişti'}</span>
                </label>
              ))}
            </fieldset>
            <pre className="kpe-diff" translate="no">{result.diff}</pre>
            <button type="button" className="btn small primary" disabled={busy || !pick.length || !follow} onClick={() => follow && void run(async () => setApplied(await peerApi.apply(follow.peer, follow.task, pick)))} data-kor-cmd="peer.apply">
              Seçilenleri bu projeye uygula
            </button>
          </>
        )
      )}
      {applied && (
        <ul className="ks-list">
          {applied.map((a) => (
            <li key={a.path} className={a.status === 'conflict' || a.status === 'failed' ? 'ks-warn' : 'ks-row'}>
              <span translate="no">{a.path}</span>
              <span className="muted">{a.status === 'written' ? 'yazıldı' : a.status === 'deleted' ? 'silindi' : a.status === 'conflict' ? 'çakışma: dosya burada değişmiş, atlandı' : `yazılamadı: ${a.detail ?? '—'}`}</span>
            </li>
          ))}
        </ul>
      )}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function Handoff({ sessionId }: { sessionId: string | null }) {
  const id = useId()
  const list = useSWR(LIST_KEY, () => peerApi.list())
  const [target, setTarget] = useState('')
  const [on, setOn] = useState(false)
  const [taken, setTaken] = useState<string | null>(null)
  const { busy, error, run } = useAction()
  const peer = target || list.data?.handoff || list.data?.peers[0]?.name || ''
  useEffect(
    () =>
      peerApi.onHandoff((e) => {
        if (e.session !== sessionId) return
        setOn(false)
        setTaken(`${e.peer} · ${e.task}`)
      }),
    [sessionId],
  )
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`} data-kor-event="kor:peer/handoff">
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        Kapanınca devret
        <span className={`ks-badge${on ? ' on' : ''}`}>{on ? 'Yansıtılıyor' : 'Kapalı'}</span>
      </h3>
      <p className="muted">Açık sohbet ve projenin kopyası seçilen makineye 30 saniyede bir yansıtılır. Bu bilgisayar kapanır ya da ağdan düşerse görev orada kaldığı yerden sürer.</p>
      {!sessionId ? (
        <p className="ks-empty">Önce bir sohbet açın.</p>
      ) : (
        <div className="ks-inline">
          <select className="text-input" value={peer} onChange={(e) => setTarget(e.target.value)} aria-label="Devredilecek makine">
            {list.data?.peers.map((p) => (
              <option key={p.name} value={p.name}>{p.name}</option>
            ))}
          </select>
          <button type="button" className={`btn small${on ? '' : ' primary'}`} disabled={busy || !peer} onClick={() => void run(async () => { await peerApi.standby(peer, sessionId, !on); setOn(!on); setTaken(null) })} data-kor-cmd="peer.standby">
            {on ? 'Durdur' : 'Yansıtmayı başlat'}
          </button>
        </div>
      )}
      {taken && <p className="ks-ok" role="status">Oturum <span translate="no">{taken}</span> görevi olarak diğer makinede sürüyor; Görevler'den izleyebilirsiniz.</p>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function Editors() {
  const id = useId()
  const setup = useSWR('acp:setup', () => acpApi.setup())
  const agents = useSWR('acp:agents', () => acpApi.agents())
  const [agent, setAgent] = useState('')
  const [prompt, setPrompt] = useState('')
  const [log, setLog] = useState('')
  const [ask, setAsk] = useState<AcpPermission | null>(null)
  const [stop, setStop] = useState<string | null>(null)
  const { busy, error, run } = useAction()
  const chosen = agent || agents.data?.[0]?.name || ''
  useEffect(
    () =>
      acpApi.onUpdate((e) => {
        const u = e.update as { sessionUpdate?: string; content?: { text?: string }; title?: string }
        if (u.sessionUpdate === 'agent_message_chunk') setLog((l) => l + (u.content?.text ?? ''))
        else if (u.sessionUpdate === 'tool_call') setLog((l) => `${l}\n> ${u.title ?? ''}\n`)
      }),
    [],
  )
  useEffect(() => acpApi.onPermission((e) => setAsk(e)), [])
  const zed = setup.data ? JSON.stringify(setup.data.zed, null, 2) : ''
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`} data-kor-event="kor:acp/update kor:acp/permission">
      <h3 id={`${id}-t`} className="kg-card-title ks-title">Editörler (ACP)</h3>
      <p className="muted">Zed ve diğer ACP editörleri Kor'u ajan olarak kullanır: araçlar, onaylar ve farklar editörün kendi arayüzünde. Zed ayarlarına ekleyin:</p>
      {setup.error && <p className="error-text" role="alert">{errorMessage(setup.error)}</p>}
      {setup.data && (
        <>
          <pre className="kpe-diff" translate="no" data-kor-cmd="acp.setup">{zed}</pre>
          <CopyButton getText={() => zed} label="Zed ayarını kopyala" />
        </>
      )}
      <h4>Alt-ajan olarak ACP ajanları</h4>
      {agents.data && agents.data.length === 0 && (
        <p className="ks-empty">Yapılandırılmış ACP ajanı yok. config.toml içine <code translate="no">[acp.agents.ad] command = "…"</code> ekleyin.</p>
      )}
      {agents.data && agents.data.length > 0 && (
        <form className="ks-form" onSubmit={(e) => { e.preventDefault(); void run(async () => { setLog(''); setStop(null); const r = await acpApi.run(chosen, prompt.trim()); setStop(r.stopReason) }) }}>
          <select className="text-input" value={chosen} onChange={(e) => setAgent(e.target.value)} aria-label="ACP ajanı" data-kor-cmd="acp.agents">
            {agents.data.map((a) => (
              <option key={a.name} value={a.name}>{a.name} ({a.command})</option>
            ))}
          </select>
          <textarea className="text-input" value={prompt} onChange={(e) => setPrompt(e.target.value)} placeholder="Alt-ajanın görevi" aria-label="Alt-ajanın görevi" maxLength={20000} />
          <button type="submit" className="btn small primary" disabled={busy || !prompt.trim()} data-kor-cmd="acp.run">
            <Play size={14} aria-hidden="true" />
            {busy ? 'Çalışıyor…' : 'Çalıştır'}
          </button>
        </form>
      )}
      {ask && (
        <div className="ks-actions" role="group" aria-label="Alt-ajan izni">
          <span className="ks-warn" translate="no">{String((ask.toolCall as { title?: string }).title ?? '')}</span>
          {ask.options.map((o) => {
            const opt = o as { optionId?: string; name?: string }
            return (
              <button key={opt.optionId} type="button" className="btn small" onClick={() => void run(async () => { await acpApi.answer(ask.id, opt.optionId ?? null); setAsk(null) })} data-kor-cmd="acp.answer">
                {opt.name}
              </button>
            )
          })}
        </div>
      )}
      {(log || stop) && <pre className="kpe-diff" translate="no">{log}{stop ? `\n[${stop}]` : ''}</pre>}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

export function PeersPanel({ sessionId }: KorPanelProps) {
  const [follow, setFollow] = useState<{ peer: string; task: string } | null>(null)
  const { run } = useAction()
  return (
    <div className="ks kr">
      <ThisMachine />
      <Peers onTasks={(peer, task) => { setFollow({ peer, task }); void run(() => peerApi.attach(peer, task)) }} />
      <Delegate follow={follow} setFollow={setFollow} />
      <Handoff sessionId={sessionId} />
      <Editors />
    </div>
  )
}
