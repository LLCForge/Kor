// Takım görünümü (§11, §12.3): kendi barındırdığınız takım sunucusuna bağlantı, üyeler, paylaşılan kurallar.
// Sunucunun kendisi (`kor server`, üye listesi, kural senkronu) Faz 8'de gelir: üyeler için sahte veri gösterilmez.
import { BookText, CircleCheck, CircleSlash, Link2, Unplug, Users } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { TeamCheck, TeamStatus } from '../../../ipc-team.ts'
import { api, errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import type { KorPanelProps } from '../registry.ts'
import { teamApi } from '../security/bridge.ts'

const TEAM_KEY = 'team:status'

function ConnectForm({ onDone }: { onDone: (s: TeamStatus) => void }) {
  const id = useId()
  const [url, setUrl] = useState('')
  const [token, setToken] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError(null)
    try {
      onDone(await teamApi.connect(url.trim(), token.trim() || null))
      setToken('')
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="ks-form" onSubmit={(e) => void submit(e)}>
      <div className="ks-field">
        <label htmlFor={`${id}-u`}>Sunucu adresi</label>
        <input id={`${id}-u`} className="text-input" type="url" value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://kor.example.com" maxLength={2048} spellCheck={false} translate="no" required />
      </div>
      <div className="ks-field">
        <label htmlFor={`${id}-k`}>Erişim belirteci (isteğe bağlı)</label>
        <input id={`${id}-k`} className="text-input" type="password" value={token} onChange={(e) => setToken(e.target.value)} maxLength={8192} autoComplete="off" spellCheck={false} />
      </div>
      <p className="muted ks-note">Adres kullanıcı ayar dosyasına, belirteç işletim sisteminin anahtarlığına yazılır. Yalnızca https; yerel sunucu için http://localhost.</p>
      {error && <p className="error-text" role="alert">{error}</p>}
      <button type="submit" className="btn small primary" disabled={busy || !url.trim()} data-kor-cmd="team.connect">
        <Link2 size={14} aria-hidden="true" />
        {busy ? 'Kaydediliyor…' : 'Bağlan'}
      </button>
    </form>
  )
}

function ServerCard({ status, onChange }: { status: TeamStatus; onChange: (s: TeamStatus) => void }) {
  const id = useId()
  const [check, setCheck] = useState<TeamCheck | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const probe = async () => {
    setBusy(true)
    setError(null)
    try {
      setCheck(await teamApi.check())
    } catch (err) {
      setCheck(null)
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  const disconnect = async () => {
    const ok = await confirmAction({ title: 'Takım sunucusu bağlantısı kesilsin mi?', message: 'Sunucu adresi ayarlardan, erişim belirteci anahtarlıktan silinir. Sunucudaki veriye dokunulmaz.', confirmLabel: 'Bağlantıyı kes', danger: true })
    if (!ok) return
    setBusy(true)
    setError(null)
    try {
      setCheck(null)
      onChange(await teamApi.disconnect())
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <Link2 size={16} aria-hidden="true" />
        Sunucu bağlantısı
        <span className={`ks-badge${status.server ? ' on' : ''}`}>{status.server ? 'Ayarlı' : 'Bağlı değil'}</span>
      </h3>
      {status.server ? (
        <>
          <dl className="ks-facts">
            <dt>Adres</dt>
            <dd translate="no">{status.server}</dd>
            <dt>Kaynak</dt>
            <dd>{status.source === 'env' ? 'KOR_TEAM_MEMORY_URL ortam değişkeni' : 'Kullanıcı ayarı'}</dd>
            <dt>Belirteç</dt>
            <dd>{status.token ? 'Anahtarlıkta kayıtlı' : 'Yok'}</dd>
          </dl>
          {check && (
            <p className={check.reachable ? 'ks-ok' : 'ks-broken'} role="status">
              {check.reachable ? <CircleCheck size={14} aria-hidden="true" /> : <CircleSlash size={14} aria-hidden="true" />}
              {check.reachable ? `Sunucu yanıt verdi (HTTP ${check.status ?? '?'}).` : 'Sunucuya ulaşılamadı.'}
              {!check.reachable && check.error && <span translate="no"> {check.error}</span>}
            </p>
          )}
          {status.privacy && <p className="ks-warn">Gizlilik modu açık: yalnızca bu bilgisayardaki sunuculara bağlanılır.</p>}
          <div className="ks-actions">
            <button type="button" className="btn small" onClick={() => void probe()} disabled={busy} data-kor-cmd="team.check">
              {busy ? 'Sınanıyor…' : 'Bağlantıyı sına'}
            </button>
            {status.source === 'config' && (
              <button type="button" className="btn small cdx-btn-danger" onClick={() => void disconnect()} disabled={busy} data-kor-cmd="team.disconnect">
                <Unplug size={14} aria-hidden="true" />
                Bağlantıyı kes
              </button>
            )}
          </div>
          {status.source === 'env' && <p className="muted ks-note">Adres ortam değişkeninden geliyor; kaldırmak için değişkeni silin.</p>}
        </>
      ) : (
        <>
          <p className="muted ks-note">Kor Server kendi sunucunuzda çalışır: paylaşılan kurallar, takım hafızası, merkezi denetim kaydı ve maliyet panosu. Bağlanmak için sunucunuzun adresini girin.</p>
          <ConnectForm onDone={onChange} />
        </>
      )}
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function MembersCard({ connected }: { connected: boolean }) {
  const id = useId()
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <Users size={16} aria-hidden="true" />
        Üyeler
      </h3>
      <p className="muted ks-empty">
        {connected
          ? 'Üye listesi ve roller takım sunucusunun üye uç noktasıyla gelecek (kor server add-member). Bu sürümde sunucudan üye okunmuyor.'
          : 'Takım sunucusuna bağlı değilsiniz. Bağlandığınızda üyeler ve rolleri burada görünür.'}
      </p>
    </section>
  )
}

function RulesCard() {
  const id = useId()
  const { data, error } = useSWR('team:rules', () => api.core.compat(), { revalidateOnFocus: false })
  const shared = (data?.instructions ?? []).filter((i) => i.source.scope === 'project')
  return (
    <section className="kg-card" aria-labelledby={`${id}-t`}>
      <h3 id={`${id}-t`} className="kg-card-title ks-title">
        <BookText size={16} aria-hidden="true" />
        Paylaşılan kurallar
      </h3>
      <p className="muted ks-note">Depoya işlenen kural dosyaları (AGENTS.md, .kor/rules, diğer araçların proje kuralları) takımdaki herkese aynı şekilde uygulanır. Sunucudan otomatik kural senkronu takım sunucusuyla gelir.</p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status" data-kor-cmd="core.compat">Kurallar okunuyor…</p>
      ) : shared.length === 0 ? (
        <p className="muted ks-empty" data-kor-cmd="core.compat">Bu projede depoyla paylaşılan kural dosyası yok. Takım kuralları için proje köküne AGENTS.md ekleyin.</p>
      ) : (
        <ul className="ks-list" aria-label="Paylaşılan kural dosyaları" data-kor-cmd="core.compat">
          {shared.map((i) => (
            <li key={i.source.path} className="ks-row">
              <code translate="no" title={i.source.path}>{i.source.path}</code>
              <span className="muted">{`${i.chars} karakter`}</span>
            </li>
          ))}
        </ul>
      )}
      {data && !data.trusted && <p className="ks-warn">Proje güvenilir olarak işaretlenmedi; proje kuralları uygulanmıyor.</p>}
    </section>
  )
}

export function TeamPanel(_: KorPanelProps) {
  const { data, error, mutate } = useSWR(TEAM_KEY, () => teamApi.status())
  const set = (s: TeamStatus) => void mutate(s, { revalidate: false })
  if (error) {
    return (
      <div className="ks" role="alert">
        <p className="error-text">{errorMessage(error)}</p>
        <button type="button" className="btn small" onClick={() => void mutate()} data-kor-cmd="team.status">Yeniden dene</button>
      </div>
    )
  }
  if (!data) return <p className="muted pad" role="status" data-kor-cmd="team.status">Takım ayarları okunuyor…</p>
  return (
    <div className="ks">
      <ServerCard status={data} onChange={set} />
      <MembersCard connected={data.server !== null} />
      <RulesCard />
    </div>
  )
}
