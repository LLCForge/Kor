// Sunucular (§29.1): bağlı paneller/sunucular, bağlama, envanter ve plan → onay → uygula → doğrula akışı.
import { RefreshCw, ShieldCheck, Trash2 } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import type { PlanRow, RunReport } from '../../../ipc-web.ts'
import { errorMessage } from '../../api.ts'
import { Action, FieldInput } from './Action.tsx'
import { webApi } from './bridge.ts'
import { buildArgs, type Field, type FormState, missing } from './forms.ts'
import { registerSection } from './section-registry.ts'

const KINDS = ['cpanel', 'whm', 'plesk', 'directadmin', 'cyberpanel', 'hestia', 'ssh'] as const

/** Görev → değer alanları (Rust `panels::task_from` ile aynı adlar). */
const TASKS: Record<string, { label: string; fields: Field[] }> = {
  ssl: { label: 'SSL kur/yenile', fields: [{ name: 'domain', label: 'Alan adı', required: true }] },
  php: { label: 'PHP sürümü', fields: [{ name: 'domain', label: 'Alan adı', required: true }, { name: 'version', label: 'Sürüm (ör. 8.3)', required: true }] },
  'php-ini': { label: 'php.ini ayarı', fields: [{ name: 'domain', label: 'Alan adı', required: true }, { name: 'key', label: 'Anahtar', required: true }, { name: 'value', label: 'Değer', required: true }] },
  subdomain: { label: 'Alt alan adı', fields: [{ name: 'sub', label: 'Alt ad', required: true }, { name: 'domain', label: 'Alan adı', required: true }, { name: 'dir', label: 'Klasör (isteğe bağlı)' }] },
  db: { label: 'Veritabanı', fields: [{ name: 'name', label: 'Veritabanı adı', required: true }, { name: 'user', label: 'Kullanıcı', required: true }] },
  email: { label: 'E-posta hesabı', fields: [{ name: 'address', label: 'Adres', required: true }, { name: 'quota', label: 'Kota (MB)' }] },
  backup: { label: 'Yedek al', fields: [{ name: 'domain', label: 'Alan adı (isteğe bağlı)' }] },
  restore: { label: 'Yedekten geri yükle', fields: [{ name: 'archive', label: 'Arşiv (~/kor-backups/…tar.gz)', required: true }, { name: 'domain', label: 'Alan adı', required: true }] },
  clean: { label: 'Disk temizliği', fields: [{ name: 'paths', label: 'Silinecek yollar (boş: yalnızca listele)', type: 'textarea' }] },
  cron: { label: 'Cron görevi', fields: [{ name: 'schedule', label: 'Zaman (5 alan)', required: true, placeholder: '*/15 * * * *' }, { name: 'command', label: 'Komut', required: true }] },
  perms: { label: 'İzinleri düzelt (755/644)', fields: [{ name: 'path', label: 'Klasör', required: true }] },
  sftp: { label: 'SFTP/FTP kullanıcısı', fields: [{ name: 'user', label: 'Kullanıcı', required: true }, { name: 'path', label: 'Klasör', required: true }, { name: 'domain', label: 'Alan adı (isteğe bağlı)' }] },
}

function TaskCard({ hosts }: { hosts: readonly string[] }) {
  const id = useId()
  const [host, setHost] = useState('')
  const [account, setAccount] = useState('')
  const [task, setTask] = useState('ssl')
  const [state, setState] = useState<FormState>({})
  const [plan, setPlan] = useState<{ plan: PlanRow[]; reversible: boolean } | null>(null)
  const [report, setReport] = useState<RunReport | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const spec = TASKS[task]!

  const args = () => {
    const values = buildArgs(spec.fields, state)
    // Rust tarafı değerleri metin olarak alır.
    const text: Record<string, string> = {}
    for (const [k, v] of Object.entries(values)) text[k] = Array.isArray(v) ? v.join('\n') : String(v)
    return { name: host, task, values: text, account: account || undefined }
  }

  const call = async (what: 'plan' | 'apply') => {
    const gaps = missing(spec.fields, state)
    if (!host || gaps.length) {
      setError(`Eksik: ${[host ? '' : 'Sunucu', ...gaps].filter(Boolean).join(', ')}`)
      return
    }
    setBusy(what)
    setError(null)
    try {
      if (what === 'plan') {
        setReport(null)
        setPlan(await webApi.hostPlan(args()))
      } else {
        setReport(await webApi.hostTask({ ...args(), approve: true }))
        setPlan(null)
      }
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(null)
    }
  }

  return (
    <details className="ks-more kw-action" open>
      <summary>Görev çalıştır (plan → onay → uygula → doğrula)</summary>
      <div className="ks-form">
        <div className="ks-inline">
          <label className="ks-field" htmlFor={`${id}-h`}>
            Sunucu
            <select id={`${id}-h`} value={host} onChange={(e) => setHost(e.target.value)}>
              <option value="">—</option>
              {hosts.map((h) => (
                <option key={h} value={h} translate="no">{h}</option>
              ))}
            </select>
          </label>
          <label className="ks-field" htmlFor={`${id}-t`}>
            Görev
            <select
              id={`${id}-t`}
              value={task}
              onChange={(e) => {
                setTask(e.target.value)
                setState({})
                setPlan(null)
              }}
            >
              {Object.entries(TASKS).map(([k, v]) => (
                <option key={k} value={k}>{v.label}</option>
              ))}
            </select>
          </label>
          <label className="ks-field" htmlFor={`${id}-a`}>
            WHM hesabı
            <input id={`${id}-a`} value={account} onChange={(e) => setAccount(e.target.value)} />
          </label>
        </div>
        {spec.fields.map((f) => (
          <FieldInput key={f.name} id={`${id}-${f.name}`} f={f} value={state[f.name]} onChange={(v) => setState((s) => ({ ...s, [f.name]: v }))} />
        ))}
        <div className="ks-actions">
          <button type="button" className="btn small primary" disabled={busy !== null} onClick={() => void call('plan')} data-kor-cmd="web.hostPlan">
            {busy === 'plan' ? 'Envanter okunuyor…' : 'Planı göster'}
          </button>
        </div>
        {plan && (
          <div className="ks-preview" role="region" aria-label="Plan ve etki tablosu">
            <table className="kw-table">
              <thead>
                <tr>
                  <th scope="col">Adım</th>
                  <th scope="col">Hedef</th>
                  <th scope="col">Etki</th>
                  <th scope="col">Geri alınır</th>
                </tr>
              </thead>
              <tbody>
                {plan.plan.map((r, i) => (
                  <tr key={i}>
                    <td>{r.step}</td>
                    <td translate="no">{r.target}</td>
                    <td>{r.impact}</td>
                    <td>{r.reversible ? 'Evet' : 'Hayır'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!plan.reversible && <p className="ks-warn">Bu işlem Kor tarafından geri alınamaz.</p>}
            <div className="ks-actions">
              <button type="button" className={`btn small ${plan.reversible ? 'primary' : 'danger'}`} disabled={busy !== null} onClick={() => void call('apply')} data-kor-cmd="web.hostTask">
                <ShieldCheck size={14} aria-hidden="true" /> {busy === 'apply' ? 'Uygulanıyor…' : 'Onayla ve uygula'}
              </button>
              <button type="button" className="btn small" onClick={() => setPlan(null)}>Vazgeç</button>
            </div>
          </div>
        )}
        {error && <p className="error-text" role="alert">{error}</p>}
        {report && (
          <div className={`ks-preview ${report.error || (report.verify && !report.verify.ok) ? 'ks-broken' : 'ks-ok'}`} role="status">
            {report.applied && <p className="ks-note">Uygulandı: <span translate="no">{report.applied}</span></p>}
            {report.verify && (
              <p className="ks-note">
                Doğrulama: <span translate="no">{report.verify.url}</span> HTTP {report.verify.status} · {report.verify.ms} ms {report.verify.ok ? '✓' : '✗'}
              </p>
            )}
            {report.rolled_back && <p className="ks-note">Doğrulama başarısız olduğu için değişiklik geri alındı.</p>}
            {report.credential && <p className="ks-note">Parola kasada: <code translate="no">{report.credential}</code></p>}
            {report.error && <p className="error-text">{report.error}</p>}
            {report.output.length > 0 && <pre className="ks-code" translate="no">{report.output.join('\n')}</pre>}
          </div>
        )}
      </div>
    </details>
  )
}

function HostsSection() {
  const { data, error, isLoading, mutate } = useSWR('web:hosts', () => webApi.hosts())
  const [removing, setRemoving] = useState<string | null>(null)
  const names = (data ?? []).filter((h) => !h.error).map((h) => h.name)
  return (
    <>
      <div className="ks-actions">
        <p className="ks-note muted kw-grow">Panel belirteci yalnızca kasada durur (kor vault add AD). Değişiklikler önce plan olarak gösterilir, onaydan sonra uygulanır ve site yanıtıyla doğrulanır.</p>
        <button type="button" className="btn small" onClick={() => void mutate()} aria-label="Listeyi yenile" title="Listeyi yenile" data-kor-cmd="web.hosts">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {isLoading && <p className="muted" role="status">Yükleniyor…</p>}
      {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
      {data && data.length === 0 && <p className="ks-empty">Bağlı sunucu yok. Aşağıdan cPanel, Plesk, DirectAdmin, CyberPanel, HestiaCP ya da SSH sunucusu bağlayın.</p>}
      {data && data.length > 0 && (
        <ul className="ks-list">
          {data.map((h) => (
            <li key={h.name} className="ks-row">
              <span translate="no">{h.name}</span>
              <span className="ks-chip" translate="no">{h.kind ?? '?'}</span>
              <span className="muted kw-grow" translate="no">{h.error ?? h.url ?? h.ssh ?? ''}</span>
              <button
                type="button"
                className="btn small"
                aria-label={`${h.name} bağlantısını kaldır`}
                title="Bağlantıyı kaldır"
                disabled={removing === h.name}
                data-kor-cmd="web.hostRemove"
                onClick={() => {
                  setRemoving(h.name)
                  void webApi.hostRemove({ name: h.name }).finally(() => {
                    setRemoving(null)
                    void mutate()
                  })
                }}
              >
                <Trash2 size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <Action
        title="Sunucu bağla"
        note="Bağlamadan önce envanter okunur; okunamazsa kaydedilmez."
        fields={[
          { name: 'name', label: 'Ad (a-z, 0-9, -)', required: true },
          { name: 'kind', label: 'Tür', type: 'select', options: KINDS, required: true },
          { name: 'url', label: 'Panel adresi (https://…:2083)' },
          { name: 'user', label: 'Kullanıcı' },
          { name: 'secret', label: 'Kasa kaydı (belirteç adı)' },
          { name: 'ssh', label: 'SSH (connections.toml kimliği ya da kullanıcı@sunucu)' },
          { name: 'account', label: 'WHM hesabı' },
          { name: 'client', label: 'Müşteri' },
          { name: 'email', label: "Let's Encrypt e-postası" },
        ]}
        run={(a) => webApi.hostConnect(a)}
        onDone={() => void mutate()}
        button="Bağla"
        data-kor-cmd="web.hostConnect"
      />
      <Action
        title="Envanter"
        fields={[
          { name: 'name', label: 'Sunucu', type: 'select', options: names },
          { name: 'all', label: 'Tüm sunucular', type: 'checkbox' },
          { name: 'account', label: 'WHM hesabı' },
        ]}
        run={async (a) => {
          const r = (await webApi.inventory(a)) as { name: string; markdown?: string; error?: string }[]
          return r.map((x) => x.markdown ?? `# ${x.name}\n${x.error ?? ''}`).join('\n\n')
        }}
        button="Oku"
        data-kor-cmd="web.inventory"
      />
      <TaskCard hosts={names} />
    </>
  )
}

registerSection({ id: 'hosts', label: 'Sunucular ve paneller', component: HostsSection })
