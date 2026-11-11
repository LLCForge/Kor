// Gömülü kor-proxy (§4.3): başlat/durdur, durum ve kullanım, ayar dosyası, erişim anahtarı (yalnızca maskeli).
import { ExternalLink, LogIn, Play, RefreshCw, RotateCcw, Square, Trash2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import type { ProxyOauthAccount } from '../../../ipc.gen.ts'
import useSWR from 'swr'
import { api, errorMessage } from '../../api.ts'
import { confirmAction } from '../../ConfirmDialog.tsx'
import { kor } from './bridge.ts'
import { ProviderLogo } from './ProviderLogo.tsx'
import { uiLocale } from '../../ui-locale.ts'

const OAUTH_PROVIDERS: { id: string; label: string }[] = [
  { id: 'claude', label: 'Claude (Claude Code)' },
  { id: 'codex', label: 'Codex (tarayıcı)' },
  { id: 'codex-device', label: 'Codex (cihaz kodu)' },
  { id: 'antigravity', label: 'Antigravity' },
  { id: 'kimi', label: 'Kimi' },
  { id: 'kimi-ai', label: 'Kimi.ai' },
  { id: 'devin', label: 'Devin' },
  { id: 'copilot', label: 'GitHub Copilot' },
]

export function ProxyTab() {
  const status = useSWR('proxy.status', () => kor.proxy.status(), { revalidateOnFocus: false })
  const config = useSWR('proxy.config', () => kor.proxy.config(), { revalidateOnFocus: false })
  const key = useSWR('proxy.key', () => kor.proxy.key(false, false), { revalidateOnFocus: false })
  const oauth = useSWR('proxy.oauthAccounts', () => kor.proxy.oauthAccounts(), { revalidateOnFocus: false })
  const routing = useSWR('proxy.routing', () => kor.proxy.routing(), { revalidateOnFocus: false })
  const [busy, setBusy] = useState(false)
  const [oauthBusy, setOauthBusy] = useState(false)
  const [oauthProvider, setOauthProvider] = useState('claude')
  const [oauthHint, setOauthHint] = useState<string | null>(null)
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null)
  const [notes, setNotes] = useState<string[]>([])

  useEffect(
    () =>
      kor.proxy.onOauth((e) => {
        if (e.kind === 'device' && e.userCode) {
          setOauthHint(`Kodu girin: ${e.userCode} · ${e.url}`)
        } else {
          setOauthHint(e.url)
        }
      }),
  []
  )

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

  const oauthLogin = async () => {
    setOauthBusy(true)
    setMsg(null)
    setOauthHint(null)
    try {
      const r = await kor.proxy.oauthLogin(oauthProvider)
      await Promise.all([oauth.mutate(), status.mutate()])
      setMsg({ ok: true, text: `${r.label} bağlandı (${r.fileName}). Çalışan ağ geçidi kimlik havuzunu yeniledi.` })
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) })
    } finally {
      setOauthBusy(false)
      setOauthHint(null)
    }
  }

  const oauthRefresh = (fileName: string) =>
    run(async () => {
      const r = await kor.proxy.oauthRefresh(fileName)
      await oauth.mutate()
      setMsg({ ok: true, text: `${r.label} belirteçleri yenilendi.` })
    })

  const oauthSetMeta = (fileName: string, weight: number, priority: number) =>
    run(async () => {
      await kor.proxy.oauthSetMeta(fileName, weight, priority)
      await Promise.all([oauth.mutate(), status.mutate()])
    }, 'Havuz ayarı kaydedildi.')

  const saveRouting = (strategy: 'round-robin' | 'weighted-round-robin' | 'fill-first') =>
    run(async () => {
      await kor.proxy.routingSet(strategy, routing.data?.requestRetry ?? 3)
      await Promise.all([routing.mutate(), status.mutate()])
    }, 'Yönlendirme güncellendi.')

  const oauthRemove = async (fileName: string) => {
    const ok = await confirmAction({
      title: 'Abonelik hesabını kaldır',
      message: `${fileName} silinecek; bu hesap ağ geçidinden çıkar.`,
      confirmLabel: 'Sil',
      danger: true,
    })
    if (!ok) return
    await run(async () => {
      await kor.proxy.oauthRemove(fileName)
      await Promise.all([oauth.mutate(), status.mutate()])
    }, 'Hesap kaldırıldı.')
  }

  const s = status.data
  const usage = s?.usage
  const c = config.data
  const accounts = oauth.data
  const credById = useMemo(
    () => new Map((status.data?.credentials ?? []).map((cr) => [cr.id, cr])),
    [status.data?.credentials],
  )
  const grouped = useMemo(() => {
    const map = new Map<string, ProxyOauthAccount[]>()
    for (const a of accounts?.accounts ?? []) {
      const list = map.get(a.provider) ?? []
      list.push(a)
      map.set(a.provider, list)
    }
    return map
  }, [accounts?.accounts])

  const poolCount = accounts?.accounts.filter((a) => a.in_pool).length ?? 0

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
      <div className="g9-details" data-kor-cmd="proxy.oauthAccounts">
        <h4 className="kp-h4">Abonelik hesapları (çoklu havuz)</h4>
        <p className="g7-note kp-small text-pretty">
          Aynı sağlayıcıya birden fazla hesap ekleyebilirsin. Ağ geçidi istekleri havuzdaki hesaplar arasında dağıtır; kota veya 429 gelince hesap beklemeye alınır ve
          sıradaki hesaba geçilir. Claude (<span className="g7-mono">claude*</span>) ve Codex (<span className="g7-mono">gpt*</span>) dosyaları doğrudan havuza girer.
        </p>
        {routing.data && (
          <div className="kp-form" data-kor-cmd="proxy.routing">
            <label className="kg-field kp-grow">
              <span>Havuz stratejisi</span>
              <select
                value={routing.data.strategy}
                disabled={busy || oauthBusy}
                onChange={(e) => void saveRouting(e.target.value as 'round-robin' | 'weighted-round-robin' | 'fill-first')}
              >
                <option value="round-robin">Sırayla — çoklu hesap, kota dolunca otomatik geçiş (önerilen)</option>
                <option value="weighted-round-robin">Ağırlıklı sıra — hesap başına ağırlık</option>
                <option value="fill-first">Önce doldur — ilk hesabı tüket, sonra diğeri</option>
              </select>
            </label>
            <span className="kp-small muted">{`Yeniden deneme turu: ${routing.data.requestRetry}`}</span>
          </div>
        )}
        {accounts && (
          <p className="g7-note kp-small">
            Kimlik dizini: <span className="g7-mono">{accounts.dir}</span>
            {poolCount > 0 ? ` · Havuzda ${poolCount} hesap` : ''}
          </p>
        )}
        <div className="kp-form">
          <label className="kg-field kp-grow">
            <span>Yeni hesap — sağlayıcı</span>
            <select value={oauthProvider} onChange={(e) => setOauthProvider(e.target.value)} disabled={oauthBusy || busy}>
              {OAUTH_PROVIDERS.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label}
                </option>
              ))}
            </select>
          </label>
          <button type="button" className="btn small primary" data-kor-cmd="proxy.oauthLogin" disabled={oauthBusy || busy} onClick={() => void oauthLogin()}>
            <LogIn size={14} aria-hidden="true" /> {oauthBusy ? 'Giriş bekleniyor…' : 'Hesap ekle'}
          </button>
        </div>
        {oauthHint && (
          <p className="muted kp-small" role="status" translate="no">
            {oauthHint}
          </p>
        )}
        {oauth.error && (
          <p className="error-text" role="alert">
            {errorMessage(oauth.error)}
          </p>
        )}
        {grouped.size > 0 ? (
          [...grouped.entries()].map(([provider, list]) => (
            <div key={provider} className="kp-oauth-group">
              <h5 className="kp-h4" translate="no">
                {provider} <span className="muted">({list.length})</span>
              </h5>
              <ul className="kp-list">
                {list.map((a) => {
                  const live = a.in_pool && a.pool_id ? credById.get(a.pool_id) : undefined
                  return (
                    <li key={a.file_name} className="kp-row-main kp-oauth-row">
                      <ProviderLogo provider={a.provider === 'codex' ? 'openai' : a.provider} />
                      <span className="kp-name" translate="no">
                        {a.label}
                      </span>
                      {!a.in_pool && <span className="badge neutral">dosyada</span>}
                      {live && live.cooling_for_secs > 0 && (
                        <span className="badge g9-health bad">{`${live.cooling_for_secs} sn kota beklemesi`}</span>
                      )}
                      {live && live.cooling_for_secs === 0 && <span className="badge g9-on">hazır</span>}
                      {a.in_pool && (
                        <label className="kp-small kg-field kp-narrow" title="Havuz ağırlığı">
                          <span className="sr-only">Ağırlık</span>
                          <input
                            type="number"
                            min={1}
                            max={100}
                            defaultValue={a.weight}
                            disabled={busy || oauthBusy}
                            onBlur={(e) => {
                              const w = Number.parseInt(e.target.value, 10)
                              if (Number.isFinite(w) && w !== a.weight) void oauthSetMeta(a.file_name, w, a.priority)
                            }}
                          />
                        </label>
                      )}
                      <span className="g7-mono kp-small" translate="no">
                        {a.file_name}
                      </span>
                      <span className="kp-grow" />
                      {live && (
                        <span className="kp-small">{`${live.success} başarılı · ${live.failed} hata`}</span>
                      )}
                      <button
                        type="button"
                        className="btn small"
                        data-kor-cmd="proxy.oauthRefresh"
                        disabled={busy || oauthBusy}
                        onClick={() => void oauthRefresh(a.file_name)}
                        title="Belirteçleri yenile"
                      >
                        <RefreshCw size={14} aria-hidden="true" />
                      </button>
                      <button
                        type="button"
                        className="btn small"
                        data-kor-cmd="proxy.oauthRemove"
                        disabled={busy || oauthBusy}
                        onClick={() => void oauthRemove(a.file_name)}
                        title="Kaldır"
                      >
                        <Trash2 size={14} aria-hidden="true" />
                      </button>
                    </li>
                  )
                })}
              </ul>
            </div>
          ))
        ) : (
          !oauth.error && <p className="muted kp-small">Henüz abonelik hesabı yok — aynı sağlayıcıya istediğin kadar hesap ekleyebilirsin.</p>
        )}
      </div>
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
