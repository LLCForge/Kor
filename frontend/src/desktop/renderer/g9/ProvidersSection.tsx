// Ayarlar > Sağlayıcılar: anahtar profilleri (maskeli), etkin/yedek anahtar, test, uç nokta ayarı, sağlık probu, yerel Ollama.
import { Activity, Check, KeyRound, Plus, Trash2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import { useSWRConfig } from 'swr'
import type { ProviderId } from '../../../core/providers.ts'
import { ProviderTile } from './BrandIcons.tsx'
import type { ProbeResult, ProviderView } from '../../ipc-g9.ts'
import { api, errorMessage } from '../api.ts'
import { CATALOG_KEY } from '../g7-bus.ts'
import { OllamaPanel } from './OllamaPanel.tsx'
import { useProviders } from './use-g9.ts'
import { VeniceWebAccount } from './VeniceWebAccount.tsx' // [g35]

type Run = (fn: () => Promise<unknown>, ok?: string) => Promise<void>

function HealthBadge({ result }: { result: ProbeResult | undefined }) {
  if (!result) return null
  return (
    <span className={`badge g9-health ${result.ok ? 'ok' : 'bad'}`} title={result.message}>
      {result.ok ? `${result.latencyMs ?? '?'} ms` : result.message}
    </span>
  )
}

function parseHeaders(text: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const line of text.split('\n')) {
    if (!line.trim()) continue
    const i = line.indexOf(':')
    if (i <= 0) throw new Error(`Başlık satırı "Ad: değer" biçiminde olmalı: ${line.trim()}`)
    out[line.slice(0, i).trim()] = line.slice(i + 1).trim()
  }
  return out
}

function EndpointEditor({ provider, run }: { provider: ProviderView; run: Run }) {
  const id = useId()
  const ep = provider.endpoint
  const [baseURL, setBaseURL] = useState(ep.baseURL ?? '')
  const [timeout, setTimeoutValue] = useState(ep.timeoutMs ? String(ep.timeoutMs / 1000) : '')
  const [concurrent, setConcurrent] = useState(ep.maxConcurrent ? String(ep.maxConcurrent) : '')
  const [headers, setHeaders] = useState(Object.entries(ep.headers ?? {}).map(([k, v]) => `${k}: ${v}`).join('\n'))

  const save = (e: FormEvent) => {
    e.preventDefault()
    void run(async () => {
      const endpoint = {
        ...(baseURL.trim() ? { baseURL: baseURL.trim() } : {}),
        ...(timeout.trim() ? { timeoutMs: Math.round(Number(timeout) * 1000) } : {}),
        ...(concurrent.trim() ? { maxConcurrent: Number(concurrent) } : {}),
        ...(headers.trim() ? { headers: parseHeaders(headers) } : {}),
      }
      await api.g9.setEndpoint(provider.id, Object.keys(endpoint).length > 0 ? endpoint : null)
    }, `${provider.label} uç nokta ayarı kaydedildi.`)
  }

  const reset = () => {
    setBaseURL('')
    setTimeoutValue('')
    setConcurrent('')
    setHeaders('')
    void run(() => api.g9.setEndpoint(provider.id, null), `${provider.label} varsayılan uç noktaya döndü.`)
  }

  return (
    <details className="g9-details">
      <summary>Uç nokta, başlıklar, zaman aşımı, eşzamanlılık</summary>
      <form className="g9-form" onSubmit={save}>
        <label htmlFor={`${id}-url`}>Temel URL</label>
        <input id={`${id}-url`} className="text-input" value={baseURL} onChange={(e) => setBaseURL(e.target.value)} placeholder={provider.defaultBaseURL} spellCheck={false} />
        <label htmlFor={`${id}-timeout`}>İlk yanıt zaman aşımı (sn)</label>
        <input id={`${id}-timeout`} className="text-input" type="number" min={1} max={600} value={timeout} onChange={(e) => setTimeoutValue(e.target.value)} placeholder="600" />
        <label htmlFor={`${id}-conc`}>Aynı anda en fazla istek</label>
        <input id={`${id}-conc`} className="text-input" type="number" min={1} max={32} value={concurrent} onChange={(e) => setConcurrent(e.target.value)} placeholder="sınırsız" />
        <label htmlFor={`${id}-headers`}>Ek başlıklar (satır başına "Ad: değer")</label>
        <textarea id={`${id}-headers`} className="text-input g9-textarea" rows={3} value={headers} onChange={(e) => setHeaders(e.target.value)} spellCheck={false} />
        <p className="g7-note">
          Temel URL https olmalı (http yalnızca localhost). Kimlik başlıkları (Authorization, x-api-key, Cookie) buradan ayarlanamaz; başlıklar ayar dosyasında düz metin
          durur, sır koyma. İstekler yalnızca bu adresin kökenine gider.
        </p>
        <div className="g7-button-row">
          <button type="submit" className="btn small">Kaydet</button>
          <button type="button" className="btn small" onClick={reset}>Varsayılana dön</button>
        </div>
      </form>
    </details>
  )
}

function AddKeyForm({ provider, run }: { provider: ProviderView; run: Run }) {
  const id = useId()
  const [label, setLabel] = useState('')
  const [key, setKey] = useState('')
  const [backup, setBackup] = useState(false)
  const submit = (e: FormEvent) => {
    e.preventDefault()
    const name = label.trim() || (provider.profiles.length === 0 ? 'Varsayılan' : `Profil ${provider.profiles.length + 1}`)
    void run(async () => {
      await api.g9.addKey(provider.id, name, key.trim(), backup)
      setKey('')
      setLabel('')
      setBackup(false)
    }, `${provider.label} anahtarı şifreli olarak kaydedildi.`)
  }
  return (
    <form className="g9-add-key" onSubmit={submit}>
      <label htmlFor={`${id}-label`} className="sr-only">Profil adı</label>
      <input id={`${id}-label`} className="text-input" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="Profil adı (ör. İş)" maxLength={60} />
      <label htmlFor={`${id}-key`} className="sr-only">{provider.label} API anahtarı</label>
      <input
        id={`${id}-key`}
        className="text-input"
        type="password"
        value={key}
        onChange={(e) => setKey(e.target.value)}
        placeholder="API anahtarı"
        autoComplete="off"
        spellCheck={false}
      />
      <label className="g9-check">
        <input type="checkbox" checked={backup} onChange={(e) => setBackup(e.target.checked)} /> Yedek
      </label>
      <button type="submit" className="btn small" disabled={key.trim().length < 8}>
        <Plus size={14} aria-hidden="true" /> Ekle
      </button>
    </form>
  )
}

function ProviderCard({ provider, health, run, ollamaEnabled }: { provider: ProviderView; health: ProbeResult | undefined; run: Run; ollamaEnabled: boolean }) {
  const titleId = useId()
  const [tests, setTests] = useState<Record<string, ProbeResult>>({})
  const test = (profileId: string) =>
    void run(async () => {
      const r = await api.g9.testKey(profileId)
      setTests((prev) => ({ ...prev, [profileId]: r }))
    })
  const status = provider.usable ? 'Kullanılabilir' : provider.keyless ? 'Kapalı' : 'Anahtar yok'

  return (
    <section className="g7-subsection g9-provider" aria-labelledby={titleId}>
      <h4 id={titleId} className="g7-h4 g9-provider-title">
        <ProviderTile id={provider.id} label={provider.label} />
        {provider.label}
        {provider.uncensored && (
          <span className="badge g9-uncensored" title="Bu sağlayıcı sansürsüz (filtresiz) modeller sunuyor">
            Sansürsüz
          </span>
        )}
        <span className={`badge ${provider.usable ? 'g9-on' : 'neutral'}`}>{status}</span>
        <HealthBadge result={health} />
        <span className="g7-mono g9-url">{provider.baseURL}</span>
      </h4>
      {provider.catalogError && <p className="error-text g9-small">{provider.catalogError}</p>}

      {provider.keyless ? (
        <>
          <label className="g9-check">
            <input
              type="checkbox"
              checked={ollamaEnabled}
              onChange={(e) =>
                void run(() => api.g9.saveSettings({ ollamaEnabled: e.target.checked }), e.target.checked ? "Yerel Ollama etkinleştirildi." : "Yerel Ollama kapatıldı.")
              }
            />
            Yerel Ollama'yı kullan (yalnızca bu makinedeki sunucu; anahtar gerekmez)
          </label>
          {ollamaEnabled && <OllamaPanel />}
        </>
      ) : (
        <>
          {provider.profiles.length > 0 && (
            <ul className="g9-profiles" aria-label={`${provider.label} anahtar profilleri`}>
              {provider.profiles.map((p) => (
                <li key={p.id} className="g9-profile">
                  <label className="g9-check">
                    <input type="radio" name={`${titleId}-active`} checked={p.active} onChange={() => void run(() => api.g9.setActiveKey(provider.id, p.id), `${p.label} etkin.`)} />
                    <span className="g9-profile-name">{p.label}</span>
                  </label>
                  <span className="g7-mono">{p.hint}</span>
                  {p.active && (
                    <span className="badge g9-on">
                      <Check size={12} aria-hidden="true" /> Etkin
                    </span>
                  )}
                  {!p.active && (
                    <label className="g9-check" title="Etkin anahtar 401 veya 429 verince sırayla denenir">
                      <input type="checkbox" checked={p.backup} onChange={(e) => void run(() => api.g9.setKeyBackup(p.id, e.target.checked))} /> Yedek
                    </label>
                  )}
                  <HealthBadge result={tests[p.id]} />
                  <span className="g9-grow" />
                  <button type="button" className="btn small" onClick={() => test(p.id)}>
                    Test et
                  </button>
                  <button type="button" className="action-btn" onClick={() => void run(() => api.g9.removeKey(p.id), `${p.label} silindi.`)} aria-label={`${p.label} anahtarını sil`} title="Sil">
                    <Trash2 size={14} aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
          )}
          <AddKeyForm provider={provider} run={run} />
        </>
      )}
      <EndpointEditor key={JSON.stringify(provider.endpoint)} provider={provider} run={run} />
    </section>
  )
}

export function ProvidersSection() {
  const { data, error, mutate } = useProviders()
  const { mutate: globalMutate } = useSWRConfig()
  const [health, setHealth] = useState<ReadonlyMap<ProviderId, ProbeResult>>(new Map())
  const [checking, setChecking] = useState(false)
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null)

  const run: Run = async (fn, ok) => {
    setMessage(null)
    try {
      await fn()
      await Promise.all([mutate(), globalMutate('state'), globalMutate(CATALOG_KEY)])
      if (ok) setMessage({ ok: true, text: ok })
    } catch (err) {
      setMessage({ ok: false, text: errorMessage(err) })
    }
  }

  const probe = async () => {
    setChecking(true)
    setMessage(null)
    try {
      const results = await api.g9.health()
      setHealth(new Map(results.map((r) => [r.provider, r])))
      if (results.length === 0) setMessage({ ok: false, text: 'Kullanılabilir sağlayıcı yok; önce bir anahtar ekle.' })
    } catch (err) {
      setMessage({ ok: false, text: errorMessage(err) })
    } finally {
      setChecking(false)
    }
  }

  if (error) return <p className="error-text">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>

  return (
    <div className="g7-stack">
      <p className="g7-note text-pretty">
        <KeyRound size={12} aria-hidden="true" /> Anahtarlar işletim sisteminin anahtarlığıyla (Windows DPAPI, macOS Keychain, Linux libsecret) şifrelenir ve yalnızca
        kendi sağlayıcısının adresine gönderilir; bu ekranda yalnızca son 4 karakter görünür. Model seçicide başka sağlayıcının modeli seçilince istek o sağlayıcıya gider.
      </p>
      {!data.persisted && (
        <p className="g9-warn" role="alert">
          Bu sistemde güvenli anahtar deposu yok (Linux'ta libsecret/kwallet gerekir): anahtarlar yalnızca uygulama açıkken bellekte tutulur.
        </p>
      )}
      {data.envVenice && <p className="g7-note">API anahtarı ortam değişkeninden geliyor.</p>}
      <div className="g7-button-row">
        <button type="button" className="btn small" onClick={() => void probe()} disabled={checking}>
          <Activity size={14} aria-hidden="true" /> {checking ? 'Yoklanıyor...' : 'Durumu yokla (gecikme ve erişilebilirlik)'}
        </button>
      </div>
      {message && (
        <p className={message.ok ? 'g7-note' : 'error-text'} role="status">
          {message.text}
        </p>
      )}
      {data.providers.map((p) => (
        <ProviderCard key={p.id} provider={p} health={health.get(p.id)} run={run} ollamaEnabled={data.settings.ollamaEnabled === true} />
      ))}
      {/* [g35] Venice web hesabı: API anahtarına alternatif giriş yolu (kendi paneli vardır). */}
      <VeniceWebAccount />
    </div>
  )
}
