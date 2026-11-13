// Komut sandbox'ı (§6.4, ADR 0033): arka uç, mod, sınırlar, ağ izin listesi, öz-denetim, son engeller.
// Rust: ns/security.rs `sandbox.*`; mod ve sınırlar `core.configSet`, son engeller `audit.list`.
import { Download, FlaskConical, RefreshCw, ShieldAlert, ShieldCheck } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { AuditEntry, SandboxMode, SandboxPreference, SandboxProbe, SandboxStatus } from '../../../ipc-security.ts'
import { api, errorMessage } from '../../api.ts'
import { auditApi, sandboxApi } from './bridge.ts'
import { limitText, parseAllowlist, parseLimit } from './sandbox-shape.ts'

const MODES: ReadonlyArray<{ id: SandboxMode; label: string }> = [
  { id: 'on', label: 'Açık: komutlar izole çalışır' },
  { id: 'ask', label: 'Sor: sandbox yok, her komut onay ister' },
  { id: 'off', label: 'Kapalı: izin modu karar verir' },
]

const PREFERENCES: ReadonlyArray<{ id: SandboxPreference; label: string }> = [
  { id: 'auto', label: 'Otomatik: işletim sisteminin kendi arka ucu, yoksa Docker' },
  { id: 'native', label: 'Yalnızca işletim sisteminin kendi arka ucu' },
  { id: 'docker', label: 'Docker' },
]

const BACKENDS: Record<string, string> = {
  bubblewrap: 'bubblewrap (Linux)',
  seatbelt: 'sandbox-exec (macOS)',
  appcontainer: 'AppContainer (Windows)',
  docker: 'Docker',
  none: 'yok',
}

type DeniedEntry = AuditEntry & { backend?: string; kind?: string; path?: string; host?: string; command?: string }

function networkText(s: SandboxStatus): string {
  if (!s.network.on) return 'Kapalı'
  if (s.network.allow.length === 0) return 'Açık (her yer)'
  const how =
    s.network.filter === 'domains'
      ? "sunucu adları uygulanır: komut yalnızca Kor'un süzen vekiline ulaşır"
      : s.network.filter === 'proxy'
        ? 'vekil ayarını kullanan programlar süzülür; doğrudan bağlananlar süzülmez'
        : 'bu arka uç sunucu süzemez: ağın tamamı açılır'
  return `İzin listesi (${how})`
}

function Limits({ data, onSaved }: { data: SandboxStatus; onSaved: () => Promise<unknown> }) {
  const id = useId()
  const [memory, setMemory] = useState(String(data.limits.memoryMb))
  const [cpu, setCpu] = useState(String(data.limits.cpuPercent))
  const [allow, setAllow] = useState(data.network.allow.join('\n'))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<string | null>(null)
  const mem = parseLimit(memory, 1_048_576)
  const pct = parseLimit(cpu, 100)
  const list = parseAllowlist(allow)

  const save = async (e: FormEvent) => {
    e.preventDefault()
    if (mem === null || pct === null || list.invalid.length > 0) return
    setBusy(true)
    setError(null)
    setDone(null)
    try {
      await api.core.configSet('sandbox.memory_mb', String(mem))
      await api.core.configSet('sandbox.cpu_percent', String(pct))
      await sandboxApi.setNetwork(list.entries)
      await onSaved()
      setDone('Kaydedildi; yeni sohbetlerde geçerli olur.')
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <details className="ks-more">
      <summary>Sınırlar ve ağ izin listesi</summary>
      <form className="ks-form" onSubmit={(e) => void save(e)}>
        <div className="ks-inline">
          <div className="ks-field">
            <label htmlFor={`${id}-m`}>Bellek (MiB, 0 = sınır yok)</label>
            <input id={`${id}-m`} className="text-input" inputMode="numeric" value={memory} onChange={(e) => setMemory(e.target.value)} aria-invalid={mem === null} />
          </div>
          <div className="ks-field">
            <label htmlFor={`${id}-c`}>CPU (%, 0 = sınır yok)</label>
            <input id={`${id}-c`} className="text-input" inputMode="numeric" value={cpu} onChange={(e) => setCpu(e.target.value)} aria-invalid={pct === null} />
          </div>
        </div>
        <div className="ks-field">
          <label htmlFor={`${id}-n`}>Ağ izin listesi (satır başına host:port; * = her yer; boş = ağ kapalı)</label>
          <textarea id={`${id}-n`} className="text-input ks-textarea" value={allow} onChange={(e) => setAllow(e.target.value)} placeholder="registry.npmjs.org:443" spellCheck={false} translate="no" aria-invalid={list.invalid.length > 0} />
        </div>
        {list.invalid.length > 0 && <p className="ks-warn" role="alert">Geçersiz: <span translate="no">{list.invalid.join(', ')}</span></p>}
        {error && <p className="error-text" role="alert">{error}</p>}
        {done && <p className="muted" role="status">{done}</p>}
        <button type="submit" className="btn small primary" disabled={busy || mem === null || pct === null || list.invalid.length > 0} data-kor-cmd="sandbox.setNetwork">
          Kaydet
        </button>
      </form>
    </details>
  )
}

function Probe() {
  const [result, setResult] = useState<SandboxProbe | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const run = async () => {
    setBusy(true)
    setError(null)
    try {
      setResult(await sandboxApi.test())
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="ks-form">
      <div className="ks-actions">
        <button type="button" className="btn small" onClick={() => void run()} disabled={busy} data-kor-cmd="sandbox.test">
          <FlaskConical size={14} aria-hidden="true" />
          {busy ? 'Deneniyor…' : 'Sandbox\'ı dene'}
        </button>
        <span className="muted ks-note">Geçici bir projenin dışına tek dosya yazmayı dener; ikisi de sonra silinir.</span>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {result && (
        <>
          <p className={result.blocked ? 'ks-ok' : 'ks-broken'} role="status">
            {result.blocked ? <ShieldCheck size={14} aria-hidden="true" /> : <ShieldAlert size={14} aria-hidden="true" />}
            {result.blocked ? `Engellendi: ${result.backend} proje dışına yazmayı durdurdu.` : `ENGELLENMEDİ: ${result.backend} altındaki komut proje dışına yazdı. Bu sandbox'a güvenmeyin.`}
          </p>
          {result.blocked && (result.detected.length > 0
            ? <p className="muted ks-note">Çıktıda tanındı, denetim kaydına geçer: <code translate="no">{result.detected[0]}</code></p>
            : <p className="ks-warn">Yazma engellendi ama mesajı tanınmadı; böyle bir engel denetim kaydına geçmez.</p>)}
        </>
      )}
    </div>
  )
}

function RecentBlocks() {
  const { data, error } = useSWR('security:sandbox-denied', () => auditApi.list({ action: 'sandbox_denied', limit: 5 }))
  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted" role="status" data-kor-cmd="audit.list">Son engeller okunuyor…</p>
  if (data.total === 0) return <p className="muted ks-empty" data-kor-cmd="audit.list">Henüz engellenen bir komut yok.</p>
  return (
    <ul className="ks-entries" aria-label="Son sandbox engelleri" data-kor-cmd="audit.list">
      {(data.entries as DeniedEntry[]).map((e) => (
        <li key={e.hash} className="ks-entry">
          <div className="ks-entry-head">
            <span className="ks-chip bad">{e.kind === 'network' ? 'ağ' : 'dosya'}</span>
            <span className="ks-chip" translate="no">{e.backend}</span>
            <time className="ks-time muted" dateTime={e.ts}>{new Date(e.ts).toLocaleString()}</time>
          </div>
          <p className="ks-entry-text" translate="no">{e.path ?? e.host ?? e.command ?? ''}</p>
        </li>
      ))}
    </ul>
  )
}

export function SandboxCard() {
  const id = useId()
  const { data, error, mutate } = useSWR('security:sandbox', () => sandboxApi.status())
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const [pulled, setPulled] = useState<string | null>(null)

  const set = async (key: string, value: string) => {
    setBusy(true)
    setActionError(null)
    try {
      await api.core.configSet(key, value)
      await mutate()
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const pull = async () => {
    setBusy(true)
    setActionError(null)
    setPulled(null)
    try {
      const r = await sandboxApi.pull()
      setPulled(r.ok ? `${r.image} indirildi.` : `${r.image} indirilemedi: ${r.output}`)
      await mutate()
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const enforcing = data?.mode === 'on' && data.enforcing
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <div className="kg-card-head">
        <h3 id={`${id}-t`} className="kg-card-title ks-title">
          {enforcing ? <ShieldCheck size={16} aria-hidden="true" /> : <ShieldAlert size={16} aria-hidden="true" />}
          Komut sandbox'ı
        </h3>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Sandbox durumunu yenile" title="Yenile" data-kor-cmd="sandbox.status">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status" data-kor-cmd="sandbox.status">Sandbox durumu okunuyor…</p>
      ) : (
        <>
          <dl className="ks-facts" data-kor-cmd="sandbox.status">
            <dt>Arka uç</dt>
            <dd translate="no">{BACKENDS[data.backend] ?? data.backend}</dd>
            <dt>Durum</dt>
            <dd>{data.mode === 'on' ? (data.enforcing ? 'Etkin' : 'Açık ama arka uç yok') : 'Kullanılmıyor'}</dd>
            <dt>Dosyalar</dt>
            <dd>{data.files ? 'Yalnızca proje ve özel geçici klasör yazılabilir' : 'Sınırlanmıyor'}</dd>
            <dt>Ağ</dt>
            <dd>{networkText(data)}{data.network.allow.length > 0 && <span translate="no">{`: ${data.network.allow.join(', ')}`}</span>}</dd>
            <dt>Bellek</dt>
            <dd>{limitText(data.limits.memoryMb, ' MiB', data.limits.memoryEnforced)}</dd>
            <dt>CPU</dt>
            <dd>{limitText(data.limits.cpuPercent, '%', data.limits.cpuEnforced, true)}</dd>
            <dt>Proje dosyaları</dt>
            <dd>{data.overlay ? 'Katman: yazılanlar atılır, proje değişmez' : 'Okuma-yazma'}</dd>
            {data.docker && (
              <>
                <dt>Docker imajı</dt>
                <dd><span translate="no">{data.image}</span>{` · ${!data.docker.daemon ? 'Docker servisine ulaşılamıyor' : data.docker.imagePresent ? 'mevcut' : 'bu makinede yok'}`}</dd>
              </>
            )}
          </dl>
          <label htmlFor={`${id}-mode`} className="ks-label">Mod</label>
          <select id={`${id}-mode`} className="g7-select" value={data.mode} disabled={busy} onChange={(e) => void set('sandbox', e.target.value)} data-kor-cmd="core.configSet">
            {MODES.map((m) => (
              <option key={m.id} value={m.id}>{m.label}</option>
            ))}
          </select>
          <label htmlFor={`${id}-pref`} className="ks-label">Arka uç tercihi</label>
          <select id={`${id}-pref`} className="g7-select" value={data.preference} disabled={busy} onChange={(e) => void set('sandbox.backend', e.target.value)} data-kor-cmd="core.configSet">
            {PREFERENCES.map((p) => (
              <option key={p.id} value={p.id}>{p.label}</option>
            ))}
          </select>
          <label className="ks-check">
            <input type="checkbox" checked={data.overlay} disabled={busy} onChange={(e) => void set('sandbox.overlay', String(e.target.checked))} data-kor-cmd="core.configSet" />
            Katman: komutlar projeyi değiştirmesin (yazılanlar atılır)
          </label>
          {data.mode === 'on' && !data.enforcing && (
            <p className="ks-warn" role="alert">Bu makinede sandbox arka ucu bulunamadı. Kor komutları sessizce sandbox'sız çalıştırmaz: her komuttan önce onay ister.</p>
          )}
          {data.notes.map((n) => (
            <p key={n} className="ks-warn" role="alert">{n}</p>
          ))}
          {data.docker?.daemon && !data.docker.imagePresent && (
            <div className="ks-actions">
              <button type="button" className="btn small" onClick={() => void pull()} disabled={busy} data-kor-cmd="sandbox.pull">
                <Download size={14} aria-hidden="true" />
                {busy ? 'İndiriliyor…' : 'İmajı indir'}
              </button>
              <span className="muted ks-note">Kor imajı kendiliğinden asla indirmez.</span>
            </div>
          )}
          {pulled && <p className="muted" role="status">{pulled}</p>}
          {data.enforcing && <Probe />}
          <Limits key={`${data.limits.memoryMb}-${data.limits.cpuPercent}-${data.network.allow.join(',')}`} data={data} onSaved={() => mutate()} />
          <h4 className="ks-label">Son engeller (denetim kaydı)</h4>
          <RecentBlocks />
          <p className="muted ks-note">Ayarlar kullanıcı ayar dosyasına yazılır ve yeni sohbetlerde geçerli olur. Sınırlananlar: Bash, PowerShell, proje kodu çalıştıran denetimler ve bench testleri; kancalar, MCP ve dil sunucuları sizin yapılandırmanızdır ve sınırlanmaz.</p>
        </>
      )}
      {actionError && <p className="error-text" role="alert">{actionError}</p>}
    </section>
  )
}
