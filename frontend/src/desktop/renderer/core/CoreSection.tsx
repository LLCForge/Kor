// Ayarlar > Çekirdek: CLI'daki `kor --privacy-mode`, `kor undo`, `kor compat status`, `kor config explain/paths/set`,
// `kor plugin marketplace` ve `kor plugin doctor` komutlarının masaüstü karşılığı (parity.toml). Mantık çekirdekte.
import { RotateCcw, ShieldCheck, Stethoscope, Trash2, Plus } from 'lucide-react'
import { useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { UndoReport } from '../../ipc-core.ts'
import { api, errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'

function useMessage() {
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null)
  return {
    message,
    ok: (text: string) => setMessage({ text, error: false }),
    fail: (err: unknown) => setMessage({ text: errorMessage(err), error: true }),
  }
}

function Message({ m }: { m: { text: string; error: boolean } | null }) {
  if (!m) return null
  return (
    <p className={m.error ? 'error-text' : 'muted'} role={m.error ? 'alert' : 'status'}>
      {m.text}
    </p>
  )
}

function PrivacyBlock() {
  const { data, mutate } = useSWR('core:privacy', () => api.core.privacy(), { revalidateOnFocus: false })
  const msg = useMessage()
  const toggle = (enabled: boolean) =>
    void api.core
      .setPrivacy(enabled)
      .then(() => mutate({ enabled }, { revalidate: false }))
      .then(() => msg.ok('Kaydedildi; yeni sohbetlerde geçerli.'))
      .catch(msg.fail)
  return (
    <section className="g7-subsection" aria-labelledby="core-privacy" data-kor-cmd="core.setPrivacy">
      <h4 className="g7-h4" id="core-privacy">
        <ShieldCheck size={16} aria-hidden="true" /> Gizlilik modu
      </h4>
      <p className="g7-section-desc">
        Açıkken yalnızca bu bilgisayarda çalışan modeller kullanılır; web, tarayıcı, MCP, SSH ve uzak katalog kapanır. CLI karşılığı: <code>kor --privacy-mode</code>.
      </p>
      <label className="core-check">
        <input type="checkbox" checked={data?.enabled ?? false} disabled={!data} onChange={(e) => toggle(e.target.checked)} /> Gizlilik modu açık
      </label>
      <Message m={msg.message} />
    </section>
  )
}

function UndoBlock() {
  const { data, mutate } = useSWR('core:undo', () => api.core.undoList(), { revalidateOnFocus: true })
  const msg = useMessage()
  const [report, setReport] = useState<UndoReport | null>(null)
  const run = async (session: string) => {
    const ok = await confirmAction({
      title: 'Oturumu geri al',
      message: 'Bu oturumun değiştirdiği tüm dosyalar ilk hallerine döner, oluşturduğu dosyalar silinir. Komutlarla yapılan değişiklikler geri alınmaz.',
      confirmLabel: 'Geri al',
      danger: true,
    })
    if (!ok) return
    try {
      setReport(await api.core.undo(session))
      msg.ok('Geri alındı.')
      void mutate()
    } catch (err) {
      msg.fail(err)
    }
  }
  return (
    <section className="g7-subsection" aria-labelledby="core-undo" data-kor-cmd="core.undo">
      <h4 className="g7-h4" id="core-undo">
        <RotateCcw size={16} aria-hidden="true" /> Geri al
      </h4>
      <p className="g7-section-desc">
        Ajanın bir oturumda değiştirdiği dosyaları tek adımda ilk hallerine döndürür. CLI karşılığı: <code>kor undo</code>.
      </p>
      {!data || data.length === 0 ? (
        <p className="muted">Geri alınacak bir şey yok.</p>
      ) : (
        <ul className="core-list">
          {data.map((j) => (
            <li key={j.session} className="core-row">
              <span className="core-row-main">
                <code>{j.session}</code> <span>{`· ${j.paths} dosya`}</span>
                {j.modifiedAt !== null && <span className="muted">{` · ${new Date(j.modifiedAt).toLocaleString()}`}</span>}
              </span>
              <button type="button" className="btn small" onClick={() => void run(j.session)}>
                <RotateCcw size={14} aria-hidden="true" /> Geri al
              </button>
            </li>
          ))}
        </ul>
      )}
      {report && (
        <p className="muted">
          {`${report.restored.length} dosya geri yüklendi, ${report.deleted.length} yeni dosya silindi`}
          {report.skipped.length > 0 && `, ${report.skipped.length} atlandı`}
          {report.failed.length > 0 && `, ${report.failed.length} başarısız`}
        </p>
      )}
      <Message m={msg.message} />
    </section>
  )
}

function CompatBlock() {
  const { data, error } = useSWR('core:compat', () => api.core.compat(), { revalidateOnFocus: false })
  if (error) return <p className="error-text">{errorMessage(error)}</p>
  const rows: Array<[string, number]> = data
    ? [
        ['Talimat dosyaları', data.instructions.length],
        ['İzin kuralları', data.permissions.allow.length + data.permissions.ask.length + data.permissions.deny.length],
        ['Kancalar', data.hooks.length],
        ['Komutlar', data.commands.length],
        ['Ajanlar', data.agents.length],
        ['Skill\'ler', data.skills.length],
        ['Yok sayma dosyaları', data.ignoreFiles.length],
      ]
    : []
  return (
    <section className="g7-subsection" aria-labelledby="core-compat" data-kor-cmd="core.compat">
      <h4 className="g7-h4" id="core-compat">Uyumluluk</h4>
      <p className="g7-section-desc">
        Kor'un bu projede Claude Code, Codex ve Cursor dosyalarından okudukları. CLI karşılığı: <code>kor compat status</code>.
      </p>
      <p className="g7-note">Kural öncelikleri, çakışmalar, MCP sunucuları ve dışa aktarma için yan panelde Görünüm &gt; Uyumluluk.</p>
      {!data ? (
        <p className="muted">Yükleniyor...</p>
      ) : (
        <>
          <p className="muted">{data.trusted ? 'Proje güvenilir: proje ayarları izin verebilir.' : 'Proje güvenilir değil: proje ayarları yalnızca kısıtlayabilir.'}</p>
          <table className="core-table">
            <tbody>
              {rows.map(([label, n]) => (
                <tr key={label}>
                  <th scope="row">{label}</th>
                  <td>{n}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {data.instructions.length > 0 && (
            <ul className="core-list">
              {data.instructions.map((i) => (
                <li key={i.source.path} className="core-row">
                  <code>{i.source.path}</code> <span className="muted">{`· ${i.source.eco} · ${i.chars} karakter`}</span>
                </li>
              ))}
            </ul>
          )}
          {[...data.warnings, ...data.untrustedSkipped].map((w) => (
            <p key={w} className="error-text">
              {w}
            </p>
          ))}
        </>
      )}
    </section>
  )
}

function ConfigBlock() {
  const { data, error, mutate } = useSWR('core:config', () => api.core.configExplain(), { revalidateOnFocus: false })
  const msg = useMessage()
  const [key, setKey] = useState('')
  const [value, setValue] = useState('')
  const submit = (e: FormEvent) => {
    e.preventDefault()
    void api.core
      .configSet(key.trim(), value.trim())
      .then((v) => {
        void mutate(v, { revalidate: false })
        msg.ok('Kaydedildi; yeni sohbetlerde geçerli.')
      })
      .catch(msg.fail)
  }
  if (error) return <p className="error-text">{errorMessage(error)}</p>
  return (
    <section className="g7-subsection" aria-labelledby="core-config" data-kor-cmd="core.configSet">
      <h4 className="g7-h4" id="core-config">Ayarlar</h4>
      <p className="g7-section-desc">
        Etkin ayarlar ve her birini hangi katmanın belirlediği. CLI karşılığı: <code>kor config explain</code>, <code>kor config paths</code>, <code>kor config set</code>.
      </p>
      {data && (
        <>
          <table className="core-table">
            <tbody>
              {(['system', 'user', 'project'] as const).map((k) => (
                <tr key={k}>
                  <th scope="row">{k === 'system' ? 'Sistem' : k === 'user' ? 'Kullanıcı' : 'Proje'}</th>
                  <td>
                    <code>{data.paths[k] ?? '(yok)'}</code>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <table className="core-table">
            <thead>
              <tr>
                <th scope="col">Anahtar</th>
                <th scope="col">Değer</th>
                <th scope="col">Kaynak</th>
              </tr>
            </thead>
            <tbody>
              {data.settings.map((s) => (
                <tr key={s.key}>
                  <td>
                    <code>{s.key}</code>
                  </td>
                  <td>
                    <code>{s.value}</code>
                  </td>
                  <td className="muted">{s.layer === 'default' ? 'varsayılan' : s.origin}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
      <form className="core-form" onSubmit={submit}>
        <label className="sr-only" htmlFor="core-config-key">
          Anahtar
        </label>
        <input id="core-config-key" className="text-input" value={key} onChange={(e) => setKey(e.target.value)} placeholder="computer.enabled" maxLength={100} />
        <label className="sr-only" htmlFor="core-config-value">
          Değer
        </label>
        <input id="core-config-value" className="text-input" value={value} onChange={(e) => setValue(e.target.value)} placeholder="true" maxLength={4000} />
        <button type="submit" className="btn small" disabled={!key.trim() || !value.trim()}>
          Kaydet
        </button>
      </form>
      <Message m={msg.message} />
    </section>
  )
}

function MarketplacesBlock() {
  const { data, mutate } = useSWR('core:markets', () => api.core.marketplaces(), { revalidateOnFocus: false })
  const msg = useMessage()
  const [name, setName] = useState('')
  const [source, setSource] = useState('')
  const [doctorId, setDoctorId] = useState('')
  const add = (e: FormEvent) => {
    e.preventDefault()
    void api.core
      .addMarketplace(name.trim(), source.trim())
      .then((v) => {
        void mutate(v, { revalidate: false })
        setName('')
        setSource('')
        msg.ok('Pazar eklendi.')
      })
      .catch(msg.fail)
  }
  const remove = async (n: string) => {
    if (!(await confirmAction({ title: 'Pazarı kaldır', message: `${n} pazarı kaldırılsın mı?`, confirmLabel: 'Kaldır', danger: true }))) return
    void api.core
      .removeMarketplace(n)
      .then((v) => void mutate(v, { revalidate: false }))
      .catch(msg.fail)
  }
  const doctor = (e: FormEvent) => {
    e.preventDefault()
    void api.core
      .pluginDoctor(doctorId.trim())
      .then((r) => (r.ok ? msg.ok('Eklenti manifesti geçerli.') : msg.fail(r.error ?? 'Geçersiz eklenti')))
      .catch(msg.fail)
  }
  return (
    <section className="g7-subsection" aria-labelledby="core-markets" data-kor-cmd="core.addMarketplace">
      <h4 className="g7-h4" id="core-markets">Eklenti pazarları</h4>
      <p className="g7-section-desc">
        Git deposu ya da yerel klasör olarak eklenen pazarlar. CLI karşılığı: <code>kor plugin marketplace</code>, <code>kor plugin doctor</code>.
      </p>
      {!data || data.length === 0 ? (
        <p className="muted">Eklenmiş pazar yok.</p>
      ) : (
        <ul className="core-list">
          {data.map((m) => (
            <li key={m.name} className="core-row">
              <span className="core-row-main">
                <strong>{m.name}</strong> <span className="muted">{`· ${m.plugins.length} eklenti`}</span>
              </span>
              <button type="button" className="btn small" onClick={() => void remove(m.name)} aria-label={`${m.name} pazarını kaldır`}>
                <Trash2 size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <form className="core-form" onSubmit={add}>
        <label className="sr-only" htmlFor="core-market-name">
          Pazar adı
        </label>
        <input id="core-market-name" className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Ad" maxLength={79} />
        <label className="sr-only" htmlFor="core-market-source">
          Git adresi ya da klasör
        </label>
        <input id="core-market-source" className="text-input" value={source} onChange={(e) => setSource(e.target.value)} placeholder="https://github.com/… ya da klasör" maxLength={500} />
        <button type="submit" className="btn small" disabled={!name.trim() || !source.trim()}>
          <Plus size={14} aria-hidden="true" /> Ekle
        </button>
      </form>
      <form className="core-form" onSubmit={doctor}>
        <label className="sr-only" htmlFor="core-doctor-id">
          Eklenti kimliği
        </label>
        <input id="core-doctor-id" className="text-input" value={doctorId} onChange={(e) => setDoctorId(e.target.value)} placeholder="eklenti ya da ad@pazar" maxLength={160} />
        <button type="submit" className="btn small" disabled={!doctorId.trim()}>
          <Stethoscope size={14} aria-hidden="true" /> Doğrula
        </button>
      </form>
      <Message m={msg.message} />
    </section>
  )
}

function UpdateBlock() {
  const { data: cfg, mutate } = useSWR('core:config', () => api.core.configExplain(), { revalidateOnFocus: false })
  const msg = useMessage()
  const [status, setStatus] = useState<{ available: string | null; phase: string; error: string | null } | null>(null)
  const channel = cfg?.settings.find((s) => s.key === 'update.channel')?.value.replaceAll('"', '') ?? 'stable'
  const check = () =>
    void api
      .g16UpdateCheck()
      .then((s) => setStatus({ available: s.available, phase: s.phase, error: s.error }))
      .catch(msg.fail)
  const setChannel = (name: 'stable' | 'beta' | 'nightly') =>
    void api.core
      .updateChannel(name)
      .then((v) => void mutate(v, { revalidate: false }))
      .catch(msg.fail)
  return (
    <section className="g7-subsection" aria-labelledby="core-update" data-kor-cmd="g16UpdateCheck">
      <h4 className="g7-h4" id="core-update">
        Güncellemeler
      </h4>
      <p className="g7-section-desc">
        Kor sürümleri GitHub'dan, imzası doğrulanarak gelir. CLI karşılığı: <code>kor update</code>, <code>kor update channel</code>.
      </p>
      <form className="core-form" onSubmit={(e) => e.preventDefault()}>
        <label htmlFor="core-update-channel">Kanal</label>
        <select id="core-update-channel" className="text-input" value={channel} onChange={(e) => setChannel(e.target.value as 'stable' | 'beta' | 'nightly')}>
          <option value="stable">stable</option>
          <option value="beta">beta</option>
          <option value="nightly">nightly</option>
        </select>
        <button type="button" className="btn small" onClick={check}>
          Denetle
        </button>
        {status?.phase === 'available' && status.available && (
          <button type="button" className="btn small primary" onClick={() => void api.g16UpdateInstall().catch(msg.fail)}>
            {`Kor ${status.available} sürümünü indir`}
          </button>
        )}
      </form>
      {status && (
        <p className={status.error ? 'error-text' : 'muted'}>
          {status.error ?? (status.phase === 'available' ? `Yeni sürüm: ${status.available}` : 'Kor güncel.')}
        </p>
      )}
      <Message m={msg.message} />
    </section>
  )
}

export function CoreSection() {
  return (
    <div className="g7-stack">
      <PrivacyBlock />
      <UndoBlock />
      <CompatBlock />
      <ConfigBlock />
      <MarketplacesBlock />
      <UpdateBlock />
    </div>
  )
}
