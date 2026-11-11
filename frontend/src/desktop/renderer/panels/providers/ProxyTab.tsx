// Gömülü kor-proxy (§4.3): başlat/durdur, durum ve kullanım, ayar dosyası, erişim anahtarı (yalnızca maskeli).
import { ExternalLink, Play, RotateCcw, Square } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import { api, errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { kor } from './bridge.ts'
import { ProviderLogo } from './ProviderLogo.tsx'
import { uiLocale } from '../../ui-locale.ts'

export function ProxyTab() {
  const status = useSWR('proxy.status', () => kor.proxy.status(), { revalidateOnFocus: false })
  const config = useSWR('proxy.config', () => kor.proxy.config(), { revalidateOnFocus: false })
  const key = useSWR('proxy.key', () => kor.proxy.key(false, false), { revalidateOnFocus: false })
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)
  const [notes, setNotes] = useState<string[]>([])

  const run = async (fn: () => Promise<unknown>, ok?: string) => {
    setBusy(true)
    setMsg(null)
    try {
      await fn()
      await Promise.all([status.mutate(), config.mutate(), key.mutate()])
      if (ok) setMsg({ ok: true, text: ok })
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) })
    } finally {
      setBusy(false)
    }
  }

  const start = () =>
    run(async () => {
      const r = await kor.proxy.start()
      setNotes([...(r.warnings ?? []), ...(r.notes ?? [])])
    }, 'Ağ geçidi çalışıyor.')

  const rotate = async () => {
    const ok = await confirmAction({
      title: 'Erişim anahtarını yenile',
      message: 'Eski anahtarı kullanan istemciler bağlanamaz; yeni anahtarı onlara yeniden vermen gerekir.',
      confirmLabel: 'Yenile',
      danger: true,
    })
    if (ok) await run(() => kor.proxy.key(true, false), 'Yeni erişim anahtarı oluşturuldu.')
  }

  const s = status.data
  const usage = s?.usage
  const c = config.data

  return (
    <section className="kp-section" aria-labelledby="kp-proxy-h">
      <div className="kp-head">
        <h3 id="kp-proxy-h" className="kp-h">
          Yerel ağ geçidi (kor-proxy)
        </h3>
        {s?.running ? (
          <button type="button" className="btn small" data-kor-cmd="proxy.stop" onClick={() => void run(() => kor.proxy.stop(), 'Ağ geçidi durduruldu.')} disabled={busy}>
            <Square size={14} aria-hidden="true" /> Durdur
          </button>
        ) : (
          <button type="button" className="btn small primary" data-kor-cmd="proxy.start" onClick={() => void start()} disabled={busy || !s}>
            <Play size={14} aria-hidden="true" /> Başlat
          </button>
        )}
      </div>
      <p className="g7-note text-pretty">
        Kor sağlayıcılarını OpenAI/Anthropic/Gemini uyumlu tek bir yerel adreste sunar. Abonelik hesabı bağlamak gelişmiş bir seçenektir ve sağlayıcı kullanım
        koşullarına tabidir; kendi sorumluluğundadır ve varsayılan değildir.
      </p>
      {status.error && (
        <p className="error-text" role="alert">
          {errorMessage(status.error)}
        </p>
      )}
      {msg && (
        <p className={msg.ok ? 'g7-note' : 'error-text'} role="status">
          {msg.text}
        </p>
      )}
      {notes.map((n) => (
        <p key={n} className="g9-warn">
          {n}
        </p>
      ))}
      {!s && !status.error && <p className="muted">Durum okunuyor...</p>}
      {s && (
        <dl className="kp-facts" data-kor-cmd="proxy.status">
          <dt>Durum</dt>
          <dd>
            <span className={`badge ${s.running ? 'g9-on' : 'neutral'}`}>{s.running ? 'Çalışıyor' : 'Kapalı'}</span>
          </dd>
          {s.url && (
            <>
              <dt>Adres</dt>
              <dd className="g7-mono" translate="no">
                {s.url}
              </dd>
            </>
          )}
          {usage && (
            <>
              <dt>İstek</dt>
              <dd>{`${usage.total_requests.toLocaleString(uiLocale())} (${usage.failure_count.toLocaleString(uiLocale())} hata)`}</dd>
              <dt>Token</dt>
              <dd>{usage.total_tokens.toLocaleString(uiLocale())}</dd>
            </>
          )}
          <dt>Erişim anahtarı</dt>
          <dd className="g7-mono" data-kor-cmd="proxy.key">
            {key.data?.masked ?? 'yok (ilk başlatmada oluşturulur)'}
          </dd>
        </dl>
      )}
      <div className="g7-button-row">
        {s?.running && s.url && (
          <button type="button" className="btn small" onClick={() => void api.openExternal(`${s.url}/management.html`).catch((e: unknown) => setMsg({ ok: false, text: errorMessage(e) }))}>
            <ExternalLink size={14} aria-hidden="true" /> Yönetim paneli
          </button>
        )}
        {key.data?.masked && (
          <button type="button" className="btn small" data-kor-cmd="proxy.key" onClick={() => void rotate()} disabled={busy}>
            <RotateCcw size={14} aria-hidden="true" /> Anahtarı yenile
          </button>
        )}
      </div>
      {s?.credentials && s.credentials.length > 0 && (
        <>
          <h4 className="kp-h4">Kimlik havuzu</h4>
          <ul className="kp-list">
            {s.credentials.map((cr) => (
              <li key={cr.id} className="kp-row-main">
                <ProviderLogo provider={cr.provider} />
                <span className="kp-name" translate="no">
                  {cr.id}
                </span>
                <span className="g7-mono">{cr.key}</span>
                <span className="kp-grow" />
                <span className="kp-small">{`${cr.success} başarılı · ${cr.failed} hata`}</span>
                {cr.cooling_for_secs > 0 && <span className="badge g9-health bad">{`${cr.cooling_for_secs} sn bekliyor`}</span>}
              </li>
            ))}
          </ul>
        </>
      )}
      {c && (
        <details className="g9-details" data-kor-cmd="proxy.config">
          <summary>Ayarlar</summary>
          <p className="g7-note">
            Dosya: <span className="g7-mono">{c.path ?? '—'}</span> {c.exists ? '' : '(yok; varsayılanlar kullanılıyor)'}
          </p>
          <pre className="kp-pre" translate="no">
            {JSON.stringify(c.config, null, 2)}
          </pre>
        </details>
      )}
    </section>
  )
}
