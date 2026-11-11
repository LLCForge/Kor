// [g35] Venice web hesabı paneli: e-posta + parola ile giriş, e-posta doğrulama kodu, oturum durumu.
// API anahtarı yolundan bağımsız: buradan giriş yapılınca `veniceweb` sağlayıcısının modelleri seçilebilir olur.
import { KeyRound, LogOut, Mail, ShieldCheck } from 'lucide-react'
import { useEffect, useId, useState, type FormEvent } from 'react'
import { useSWRConfig } from 'swr'
import { api, errorMessage } from '../api.ts'
import { CATALOG_KEY } from '../g7-bus.ts'
import { PROVIDERS_KEY } from './use-g9.ts'

const STATUS_KEY = 'g35:status'

export function VeniceWebAccount() {
  const { mutate: globalMutate } = useSWRConfig()
  const id = useId()
  const [view, setView] = useState<Awaited<ReturnType<typeof api.g35.status>> | null>(null)
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [code, setCode] = useState('')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null)

  const refresh = async () => {
    const next = await api.g35.status()
    setView(next)
    setEmail((prev) => prev || next.lastEmail || '')
  }

  useEffect(() => {
    void refresh().catch(() => undefined)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const afterChange = async () => {
    await Promise.all([globalMutate(STATUS_KEY), globalMutate(PROVIDERS_KEY), globalMutate(CATALOG_KEY), globalMutate('state')])
    await refresh()
  }

  const submitPassword = (e: FormEvent) => {
    e.preventDefault()
    void (async () => {
      setBusy(true)
      setMessage(null)
      try {
        const result = await api.g35.login(email.trim(), password)
        setPassword('')
        if (result.status === 'needs_second_factor') {
          setMessage({
            ok: true,
            text:
              result.safeIdentifier
                ? `Kod ${result.safeIdentifier} adresine gönderildi (${result.strategy === 'totp' ? 'authenticator' : 'e-posta'}). Kodu gir.`
                : 'İkinci adım kodu gerekiyor. Kodu gir.',
          })
        } else {
          setMessage({ ok: true, text: 'Giriş başarılı; web hesabı model seçicide hazır.' })
        }
        await afterChange()
      } catch (err) {
        setMessage({ ok: false, text: errorMessage(err) })
      } finally {
        setBusy(false)
      }
    })()
  }

  const submitCode = (e: FormEvent) => {
    e.preventDefault()
    void (async () => {
      setBusy(true)
      setMessage(null)
      try {
        await api.g35.submitCode(code.trim())
        setCode('')
        setMessage({ ok: true, text: 'Doğrulandı; oturum açıldı.' })
        await afterChange()
      } catch (err) {
        setMessage({ ok: false, text: errorMessage(err) })
      } finally {
        setBusy(false)
      }
    })()
  }

  const logout = () => {
    void (async () => {
      setBusy(true)
      try {
        await api.g35.logout()
        setMessage({ ok: true, text: 'Çıkış yapıldı.' })
        await afterChange()
      } catch (err) {
        setMessage({ ok: false, text: errorMessage(err) })
      } finally {
        setBusy(false)
      }
    })()
  }

  if (!view) return <p className="muted">Yükleniyor...</p>

  const signedIn = view.ready

  return (
    <section className="g7-subsection g9-provider" aria-labelledby={`${id}-title`}>
      <h4 id={`${id}-title`} className="g7-h4">
        Venice web hesabı
        <span className={`badge ${signedIn ? 'g9-on' : 'neutral'}`}>{signedIn ? 'Oturum açık' : view.pendingSecondFactor ? 'Kod bekliyor' : 'Kapalı'}</span>
        <span className="g7-mono g9-url">{view.transport === 'wreq' ? 'Chrome parmak izi (wreq)' : 'düz fetch'}</span>
      </h4>
      <p className="g7-note text-pretty">
        <KeyRound size={12} aria-hidden="true" /> API anahtarı yerine Venice hesabıyla gir: e-posta + parola, hesabında 2FA varsa e-postana gelen kod. Oturum çerezleri ve
        kısa ömürlü jeton diskte tutulur; parola hiçbir yere yazılmaz. Bu yol <b>araç çağırmayan</b> düz sohbettir (web uç noktası function-calling kabul etmez).
      </p>

      {signedIn ? (
        <div className="g9-profile">
          <span className="g9-profile-name">{view.email ?? '(e-posta yok)'}</span>
          <span className="badge g9-on">
            <ShieldCheck size={12} aria-hidden="true" /> Hazır
          </span>
          <span className="g9-grow" />
          <button type="button" className="btn small" onClick={logout} disabled={busy}>
            <LogOut size={14} aria-hidden="true" /> Çıkış
          </button>
        </div>
      ) : view.pendingSecondFactor ? (
        <form className="g9-form" onSubmit={submitCode}>
          <label htmlFor={`${id}-code`}>
            <Mail size={12} aria-hidden="true" /> {view.pendingHint ?? 'Doğrulama kodu'}
            {view.pendingTarget ? ` — ${view.pendingTarget}` : ''}
          </label>
          <input
            id={`${id}-code`}
            className="text-input"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            inputMode="numeric"
            autoComplete="one-time-code"
            placeholder="123456"
            spellCheck={false}
          />
          <div className="g7-button-row">
            <button type="submit" className="btn small" disabled={busy || !/^\d{4,8}$/.test(code.trim())}>
              Doğrula
            </button>
            <button type="button" className="btn small" onClick={() => setView({ ...view, pendingSecondFactor: false })} disabled={busy}>
              Parolayı yeniden gir
            </button>
          </div>
        </form>
      ) : (
        <form className="g9-form" onSubmit={submitPassword}>
          <label htmlFor={`${id}-email`}>E-posta</label>
          <input
            id={`${id}-email`}
            className="text-input"
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            autoComplete="username"
            placeholder="ornek@eposta.com"
            spellCheck={false}
          />
          <label htmlFor={`${id}-pass`}>Parola</label>
          <input
            id={`${id}-pass`}
            className="text-input"
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="current-password"
            placeholder="••••••••"
          />
          <div className="g7-button-row">
            <button type="submit" className="btn small" disabled={busy || !email.trim() || !password}>
              {busy ? 'Giriş yapılıyor...' : 'Giriş yap'}
            </button>
          </div>
        </form>
      )}

      {message && (
        <p className={message.ok ? 'g7-note' : 'error-text'} role="status">
          {message.text}
        </p>
      )}
      {view.lastError && !message && <p className="error-text g9-small">{view.lastError}</p>}
    </section>
  )
}
