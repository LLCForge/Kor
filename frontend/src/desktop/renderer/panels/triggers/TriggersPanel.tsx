// Tetikleyiciler görünümü (§8.4, §11.3, ADR 0039): kor daemon durumu ve başlat/durdur, bekleyen onaylar,
// tetikleyici listesi (dene / düzenle / sil), ekleme formu, iş geçmişi, bildirim kanalları ve yerel git kancası.
import { Bell, Check, GitBranch, Pencil, Play, Plus, Power, RefreshCw, ScrollText, Send, Trash2, X, Zap } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import type { NotifySettings, TriggerInput, TriggerRow } from '../../../ipc-triggers.ts'
import { errorMessage } from '../../api.ts'
import type { KorPanelProps } from '../registry.ts'
import { vaultApi } from '../security/bridge.ts'
import { triggersApi } from './bridge.ts'
import { EVENT_LABEL, STATUS_LABEL, duration, formFrom, formProblem, forSave, usd } from './shape.ts'

const LIST = 'triggers:list'
const STATUS = 'triggers:daemon'
const HISTORY = 'triggers:history'
const NOTIFY = 'triggers:notify'

function useAction() {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const run = async (f: () => Promise<string | null | void>) => {
    setBusy(true)
    setError(null)
    setNote(null)
    try {
      const n = await f()
      if (n) setNote(n)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return { busy, error, note, run }
}

function DaemonCard() {
  const id = useId()
  const { data, error, mutate } = useSWR(STATUS, () => triggersApi.daemonStatus(), { refreshInterval: 5000 })
  const act = useAction()
  const [logs, setLogs] = useState<string | null>(null)
  const running = data?.running === true
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <Power size={16} aria-hidden="true" />
        kor daemon
        <span className={`ks-badge${running ? ' on' : ''}`}>{running ? 'Çalışıyor' : 'Durdu'}</span>
      </h3>
      {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
      {!data && !error && <p className="muted" role="status" data-kor-cmd="triggers.daemonStatus">Daemon durumu okunuyor…</p>}
      {running && (
        <dl className="ks-facts">
          <dt>Adres</dt>
          <dd translate="no">http://{data?.addr}</dd>
          <dt>Etkin tetikleyici</dt>
          <dd>{data?.triggers ?? '—'}</dd>
          <dt>Kuyruk</dt>
          <dd>{data?.queued ?? '—'}</dd>
        </dl>
      )}
      {!running && data && <p className="muted ks-note">Daemon tetikleyicileri çalıştırır; webhook dinleyicisi yalnızca bu bilgisayarda (127.0.0.1) açılır. Uygulama kapalıyken de çalışır.</p>}
      <div className="kt-actions">
        {running ? (
          <button type="button" className="btn small" disabled={act.busy} onClick={() => void act.run(async () => { await triggersApi.daemonStop(); await mutate() })} data-kor-cmd="triggers.daemonStop">
            <Power size={14} aria-hidden="true" />Durdur
          </button>
        ) : (
          <button type="button" className="btn small primary" disabled={act.busy} onClick={() => void act.run(async () => { await triggersApi.daemonStart(); await mutate() })} data-kor-cmd="triggers.daemonStart">
            <Play size={14} aria-hidden="true" />Başlat
          </button>
        )}
        <button type="button" className="btn small" disabled={act.busy} onClick={() => void act.run(async () => setLogs(logs == null ? await triggersApi.daemonLogs(200) : null))} data-kor-cmd="triggers.daemonLogs">
          <ScrollText size={14} aria-hidden="true" />{logs == null ? 'Günlüğü göster' : 'Günlüğü gizle'}
        </button>
        <button type="button" className="btn small" disabled={act.busy} onClick={() => void act.run(async () => `Oturum açılışında başlar: ${await triggersApi.daemonInstall()}`)} data-kor-cmd="triggers.daemonInstall">
          Oturum açılışında başlat
        </button>
        <button type="button" className="btn small" disabled={act.busy} onClick={() => void act.run(async () => { await triggersApi.daemonUninstall(); return 'Oturum açılışı kaydı kaldırıldı' })} data-kor-cmd="triggers.daemonUninstall">
          Açılış kaydını kaldır
        </button>
      </div>
      {logs != null && <pre className="kt-log" translate="no">{logs || 'Günlük boş.'}</pre>}
      {act.note && <p className="ks-ok" role="status">{act.note}</p>}
      {act.error && <p className="error-text" role="alert">{act.error}</p>}
      {running && <Waiting />}
    </section>
  )
}

function Waiting() {
  const { data, mutate } = useSWR(STATUS, () => triggersApi.daemonStatus())
  const act = useAction()
  const waiting = data?.waiting ?? []
  if (!waiting.length) return null
  const answer = (id: string, ok: boolean) => act.run(async () => { await triggersApi.answer(id, ok); await mutate() })
  return (
    <div className="kt-waiting" role="region" aria-label="Bekleyen onaylar">
      <h4 className="ks-label">Onay bekleyenler</h4>
      <ul className="kt-list">
        {waiting.map((w) => (
          <li key={w.id} className="kt-row">
            <span className="kt-grow" translate="no">{w.summary}</span>
            <button type="button" className="btn small primary" disabled={act.busy} onClick={() => void answer(w.id, true)} data-kor-cmd="triggers.answer">
              <Check size={14} aria-hidden="true" />Onayla
            </button>
            <button type="button" className="btn small" disabled={act.busy} onClick={() => void answer(w.id, false)} data-kor-cmd="triggers.answer">
              <X size={14} aria-hidden="true" />Reddet
            </button>
          </li>
        ))}
      </ul>
      {act.error && <p className="error-text" role="alert">{act.error}</p>}
    </div>
  )
}

function TriggerForm({ initial, onDone }: { initial: TriggerRow | null; onDone: () => void }) {
  const id = useId()
  const [f, setF] = useState<TriggerInput>(formFrom(initial))
  const [scope, setScope] = useState<'project' | 'user'>(initial && !initial.origin.includes('.kor') ? 'user' : 'project')
  const act = useAction()
  const set = (p: Partial<TriggerInput>) => setF((x) => ({ ...x, ...p }))
  const problem = formProblem(f)
  const field = (key: keyof TriggerInput, label: string, placeholder = '') => (
    <>
      <label htmlFor={`${id}-${key}`} className="ks-label">{label}</label>
      <input id={`${id}-${key}`} className="text-input" value={String(f[key] ?? '')} placeholder={placeholder} onChange={(e) => set({ [key]: e.target.value } as Partial<TriggerInput>)} />
    </>
  )
  return (
    <form
      className="kt-form"
      onSubmit={(e) => {
        e.preventDefault()
        if (problem) return
        void act.run(async () => {
          await triggersApi.save(forSave(f), scope)
          onDone()
        })
      }}
    >
      {field('name', 'Ad', 'ci')}
      <label htmlFor={`${id}-ev`} className="ks-label">Olay</label>
      <select id={`${id}-ev`} className="g7-select" value={f.event} onChange={(e) => set({ event: e.target.value as TriggerInput['event'] })}>
        {Object.entries(EVENT_LABEL).map(([k, v]) => <option key={k} value={k}>{v}</option>)}
      </select>
      {f.event === 'git_push' && field('branch', 'Dal (boş = hepsi)', 'main')}
      {f.event === 'git_push' && field('secret', 'Forge webhook sırrı (kasadaki ad, isteğe bağlı)', 'PUSH_HOOK_SECRET')}
      {f.event === 'cron' && field('schedule', 'Zamanlama (dakika saat gün ay haftanın-günü)', '0 3 * * *')}
      {f.event === 'webhook' && field('path', 'Yol', '/hooks/deploy')}
      {f.event === 'webhook' && field('secret', 'HMAC sırrı (kasadaki ad)', 'DEPLOY_HOOK_SECRET')}
      {f.event === 'log_match' && field('source', 'Kaynak (dosya, journald:birim, docker:kapsayıcı)', 'logs/nginx.log')}
      {f.event === 'log_match' && field('pattern', 'Desen (düzenli ifade)', '\\s5\\d\\d\\s')}
      <label htmlFor={`${id}-task`} className="ks-label">Görev</label>
      <textarea id={`${id}-task`} className="text-input" rows={3} value={f.task} onChange={(e) => set({ task: e.target.value })} placeholder="testleri çalıştır, hataları düzelt, PR aç" />
      {field('model', 'Model (boş = varsayılan)', 'anthropic/claude-sonnet-4-5')}
      <label htmlFor={`${id}-budget`} className="ks-label">Çalışma başına bütçe (USD, 0 = sınırsız)</label>
      <input id={`${id}-budget`} className="text-input" type="number" min={0} step={0.1} value={f.budgetUsd ?? 0} onChange={(e) => set({ budgetUsd: Number(e.target.value) })} />
      <label htmlFor={`${id}-appr`} className="ks-label">İzin istekleri</label>
      <select id={`${id}-appr`} className="g7-select" value={f.approval} onChange={(e) => set({ approval: e.target.value as TriggerInput['approval'] })}>
        <option value="notify">Her biri için bildirimle onay iste</option>
        <option value="auto">Yıkıcı olmayanlara izin ver, yıkıcıları sor</option>
        <option value="deny">Hepsini reddet</option>
      </select>
      <label className="ks-check"><input type="checkbox" checked={f.pr} onChange={(e) => set({ pr: e.target.checked })} />Sonucu kor/… dalına commit et, gönder ve PR aç</label>
      <label className="ks-check"><input type="checkbox" checked={f.enabled} onChange={(e) => set({ enabled: e.target.checked })} />Etkin</label>
      <label htmlFor={`${id}-scope`} className="ks-label">Kaydedileceği yer</label>
      <select id={`${id}-scope`} className="g7-select" value={scope} disabled={initial != null} onChange={(e) => setScope(e.target.value as 'project' | 'user')}>
        <option value="project">Bu proje (.kor/config.toml)</option>
        <option value="user">Kullanıcı ayarı</option>
      </select>
      {problem && <p className="ks-warn" role="status">{problem}</p>}
      <div className="kt-actions">
        <button type="submit" className="btn small primary" disabled={act.busy || problem != null} data-kor-cmd="triggers.save">
          <Check size={14} aria-hidden="true" />Kaydet
        </button>
        <button type="button" className="btn small" onClick={onDone}>Vazgeç</button>
      </div>
      {act.error && <p className="error-text" role="alert">{act.error}</p>}
    </form>
  )
}

function TriggerList() {
  const id = useId()
  const { data, error, isLoading, mutate } = useSWR(LIST, () => triggersApi.list())
  const [editing, setEditing] = useState<TriggerRow | 'new' | null>(null)
  const act = useAction()
  const done = () => {
    setEditing(null)
    void mutate()
  }
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title"><Zap size={16} aria-hidden="true" />Tetikleyiciler</h3>
        <button type="button" className="action-btn" onClick={() => setEditing('new')} aria-label="Tetikleyici ekle" title="Tetikleyici ekle" data-kor-cmd="triggers.save">
          <Plus size={14} aria-hidden="true" />
        </button>
      </div>
      {editing && <TriggerForm initial={editing === 'new' ? null : editing} onDone={done} />}
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : isLoading || !data ? (
        <p className="muted" role="status" data-kor-cmd="triggers.list">Tetikleyiciler okunuyor…</p>
      ) : !data.length ? (
        <p className="ks-empty" data-kor-cmd="triggers.list">Tetikleyici yok. git push, zamanlama, webhook ya da günlük eşleşmesiyle çalışan bir görev ekleyin.</p>
      ) : (
        <ul className="kt-list" data-kor-cmd="triggers.list">
          {data.map((t) => (
            <li key={`${t.origin}:${t.name}`} className={`kt-item${t.error ? ' broken' : ''}`}>
              <div className="kt-row">
                <strong className="kt-grow" translate="no">{t.name}</strong>
                <span className="ks-badge">{EVENT_LABEL[t.event]}</span>
                {!t.enabled && <span className="ks-badge">Kapalı</span>}
                <button type="button" className="action-btn" disabled={act.busy || t.error != null} onClick={() => void act.run(async () => { const r = await triggersApi.test(t.name); return r.job ? `Kuyruğa alındı: ${r.job}` : 'Deneme bu süreçte başladı' })} aria-label={`${t.name} tetikleyicisini dene`} title="Şimdi çalıştır" data-kor-cmd="triggers.test">
                  <Play size={14} aria-hidden="true" />
                </button>
                <button type="button" className="action-btn" onClick={() => setEditing(t)} aria-label={`${t.name} tetikleyicisini düzenle`} title="Düzenle" data-kor-cmd="triggers.save">
                  <Pencil size={14} aria-hidden="true" />
                </button>
                <button type="button" className="action-btn" disabled={act.busy} onClick={() => void act.run(async () => { await triggersApi.remove(t.name, t.origin); await mutate() })} aria-label={`${t.name} tetikleyicisini sil`} title="Sil" data-kor-cmd="triggers.remove">
                  <Trash2 size={14} aria-hidden="true" />
                </button>
              </div>
              <span className="muted" translate="no">{t.on}{t.pr ? ' · PR' : ''}{t.budgetUsd ? ` · $${t.budgetUsd}` : ''}</span>
              <span translate="no">{t.task}</span>
              {t.error && <span className="error-text">{t.error}</span>}
            </li>
          ))}
        </ul>
      )}
      {act.note && <p className="ks-ok" role="status">{act.note}</p>}
      {act.error && <p className="error-text" role="alert">{act.error}</p>}
    </section>
  )
}

function History() {
  const id = useId()
  const { data, error, mutate } = useSWR(HISTORY, () => triggersApi.history(null, 50), { refreshInterval: 5000 })
  const now = Date.now()
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title"><ScrollText size={16} aria-hidden="true" />Geçmiş</h3>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Geçmişi yenile" title="Yenile" data-kor-cmd="triggers.history">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status">İşler okunuyor…</p>
      ) : !data.length ? (
        <p className="ks-empty">Henüz tetikleyici işi yok.</p>
      ) : (
        <ul className="kt-list" data-kor-cmd="triggers.history">
          {data.map((j) => (
            <li key={j.id} className={`kt-item${j.status === 'failed' ? ' broken' : ''}`}>
              <div className="kt-row">
                <strong className="kt-grow" translate="no">{j.trigger}</strong>
                <span className={`ks-badge${j.status === 'done' ? ' on' : ''}`}>{STATUS_LABEL[j.status]}</span>
                <span className="muted">{duration(j.started, j.finished, now)} · {usd(j.cost_usd, j.cost_estimated)}</span>
              </div>
              <span className="muted" translate="no">{j.detail.split('\n')[0]}</span>
              {j.summary && <span translate="no">{j.summary}</span>}
              {j.pr_url && <a href={j.pr_url} target="_blank" rel="noreferrer noopener" translate="no">{j.pr_url}</a>}
              {j.error && <span className="error-text">{j.error}</span>}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

const SECRET_LABEL: Record<keyof NotifySettings['secrets'], string> = {
  KOR_TELEGRAM_TOKEN: 'Telegram bot belirteci',
  KOR_SLACK_WEBHOOK: 'Slack gelen webhook adresi',
  KOR_SMTP_PASSWORD: 'SMTP parolası',
}

function NotifyCard() {
  const id = useId()
  const { data, error, mutate } = useSWR(NOTIFY, () => triggersApi.notifyGet())
  const [draft, setDraft] = useState<Partial<NotifySettings>>({})
  const [secret, setSecret] = useState<{ name: keyof NotifySettings['secrets']; value: string }>({ name: 'KOR_TELEGRAM_TOKEN', value: '' })
  const act = useAction()
  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted" role="status" data-kor-cmd="triggers.notifyGet">Bildirim ayarları okunuyor…</p>
  const v = { ...data, ...draft }
  const text = (key: 'telegram_chat' | 'email_to' | 'email_from' | 'smtp_host' | 'smtp_user' | 'publicUrl', label: string, placeholder = '') => (
    <>
      <label htmlFor={`${id}-${key}`} className="ks-label">{label}</label>
      <input id={`${id}-${key}`} className="text-input" value={v[key]} placeholder={placeholder} onChange={(e) => setDraft((d) => ({ ...d, [key]: e.target.value }))} />
    </>
  )
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title"><Bell size={16} aria-hidden="true" />Bildirimler ve onay</h3>
      <p className="muted ks-note">Bir görev yıkıcı bir adımda ya da dosya incelemesinde durduğunda Onayla / Reddet / Detay düğmeleriyle bildirim gider; ilk yanıt görevi sürdürür. Gizlilik modunda bildirim gönderilmez.</p>
      {text('telegram_chat', 'Telegram sohbet kimliği', '123456789')}
      <label className="ks-check"><input type="checkbox" checked={v.slack} onChange={(e) => setDraft((d) => ({ ...d, slack: e.target.checked }))} />Slack'e gönder</label>
      {text('email_to', 'E-posta alıcıları (virgülle)', 'ben@ornek.com')}
      {text('email_from', 'Gönderen', 'kor@ornek.com')}
      {text('smtp_host', 'SMTP sunucusu', 'smtp.ornek.com')}
      <label htmlFor={`${id}-port`} className="ks-label">SMTP bağlantı noktası (465 TLS, 587 STARTTLS)</label>
      <input id={`${id}-port`} className="text-input" type="number" min={1} max={65535} value={v.smtp_port} onChange={(e) => setDraft((d) => ({ ...d, smtp_port: Number(e.target.value) }))} />
      {text('smtp_user', 'SMTP kullanıcısı')}
      {text('publicUrl', 'Onay bağlantıları için genel adres (boş = yalnızca metin)', 'https://kor.ornek.com')}
      <div className="kt-actions">
        <button type="button" className="btn small primary" disabled={act.busy || !Object.keys(draft).length} onClick={() => void act.run(async () => { await triggersApi.notifySet(draft); setDraft({}); await mutate(); return 'Kaydedildi' })} data-kor-cmd="triggers.notifySet">
          <Check size={14} aria-hidden="true" />Kaydet
        </button>
        <button type="button" className="btn small" disabled={act.busy} onClick={() => void act.run(async () => { const r = await triggersApi.notifyTest(); return [r.sent.length ? `Gönderildi: ${r.sent.join(', ')}` : '', ...r.errors, ...r.notes].filter(Boolean).join('\n') })} data-kor-cmd="triggers.notifyTest">
          <Send size={14} aria-hidden="true" />Deneme gönder
        </button>
      </div>
      <h4 className="ks-label">Kasadaki sırlar</h4>
      <ul className="kt-list">
        {(Object.keys(SECRET_LABEL) as Array<keyof NotifySettings['secrets']>).map((k) => (
          <li key={k} className="kt-row">
            <span className="kt-grow">{SECRET_LABEL[k]} <code translate="no">{k}</code></span>
            <span className={`ks-badge${data.secrets[k] ? ' on' : ''}`}>{data.secrets[k] ? 'Var' : 'Yok'}</span>
          </li>
        ))}
      </ul>
      <form
        className="kt-row"
        onSubmit={(e) => {
          e.preventDefault()
          if (!secret.value) return
          void act.run(async () => {
            await vaultApi.add(secret.name, secret.value)
            setSecret((s) => ({ ...s, value: '' }))
            await mutate()
            return 'Kasaya kaydedildi'
          })
        }}
      >
        <select className="g7-select" aria-label="Sır" value={secret.name} onChange={(e) => setSecret({ name: e.target.value as keyof NotifySettings['secrets'], value: '' })}>
          {Object.entries(SECRET_LABEL).map(([k, label]) => <option key={k} value={k}>{label}</option>)}
        </select>
        <input className="text-input kt-grow" type="password" autoComplete="off" aria-label="Sırrın değeri" value={secret.value} onChange={(e) => setSecret((s) => ({ ...s, value: e.target.value }))} />
        <button type="submit" className="btn small" disabled={act.busy || !secret.value} data-kor-cmd="vault.add">Kasaya kaydet</button>
      </form>
      {act.note && <p className="ks-ok" role="status">{act.note}</p>}
      {act.error && <p className="error-text" role="alert">{act.error}</p>}
    </section>
  )
}

function HookCard() {
  const id = useId()
  const [repo, setRepo] = useState('')
  const act = useAction()
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title"><GitBranch size={16} aria-hidden="true" />Yerel git kancası</h3>
      <p className="muted ks-note">git_push tetikleyicileri için: çıplak bir depoya (yerel origin ya da ayna) post-receive, çalışma deposuna reference-transaction kancası kurulur. Kanca yalnızca daemon'un gelen kutusuna küçük bir dosya bırakır; push'u hiç durdurmaz. Boş bırakılırsa açık proje.</p>
      <label htmlFor={`${id}-repo`} className="ks-label">Depo klasörü</label>
      <input id={`${id}-repo`} className="text-input" value={repo} placeholder="/srv/git/uygulama.git" onChange={(e) => setRepo(e.target.value)} />
      <div className="kt-actions">
        <button type="button" className="btn small primary" disabled={act.busy} onClick={() => void act.run(async () => `Kuruldu: ${await triggersApi.hookInstall(repo.trim() || null)}`)} data-kor-cmd="triggers.hookInstall">
          <GitBranch size={14} aria-hidden="true" />Kancayı kur
        </button>
        <button type="button" className="btn small" disabled={act.busy} onClick={() => void act.run(async () => ((await triggersApi.hookUninstall(repo.trim() || null)) ? 'Kanca kaldırıldı' : 'Bu depoda Kor kancası yok'))} data-kor-cmd="triggers.hookUninstall">
          <Trash2 size={14} aria-hidden="true" />Kaldır
        </button>
      </div>
      {act.note && <p className="ks-ok" role="status">{act.note}</p>}
      {act.error && <p className="error-text" role="alert">{act.error}</p>}
    </section>
  )
}

export function TriggersPanel(_: KorPanelProps) {
  return (
    <div className="ks">
      <div className="kg-card-head">
        <h2 className="ks-title">Tetikleyiciler</h2>
      </div>
      <DaemonCard />
      <TriggerList />
      <History />
      <NotifyCard />
      <HookCard />
    </div>
  )
}
