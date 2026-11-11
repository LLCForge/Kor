// Ayarlar > Maliyet: görev maliyet tavanı, düşük maliyet modu, en ucuz yeterli model, parametre ön ayarları,
// fiyat tablosu, yerel yanıt önbelleği ve model yetenek matrisi.
import { Trash2, Zap } from 'lucide-react'
import { useId, useMemo, useState, type FormEvent, type ReactNode } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import { sortCatalog } from '../../../core/model-catalog.ts'
import { cheapestCapable, type CapabilityNeeds } from '../../../core/model-registry-g9.ts'
import type { ParamPreset, PriceEntry } from '../../../core/settings-g9.ts'
import type { G9SettingsPatch } from '../../ipc-g9.ts'
import { api, errorMessage } from '../api.ts'
import { CATALOG_KEY, useCatalog } from '../g7-bus.ts'
import { formatTokens, formatUsd } from '../g7-format.ts'
import { CapabilityMatrix } from './CapabilityMatrix.tsx'
import { ProviderBadge } from './ProviderBadge.tsx'
import { formatCost, PROVIDERS_KEY, useProviders } from './use-g9.ts'

const CONTEXT_STEPS = [0, 32_000, 128_000, 200_000, 1_000_000] as const

function Sub({ title, children }: { title: string; children: ReactNode }) {
  const id = useId()
  return (
    <section className="g7-subsection" aria-labelledby={id}>
      <h4 id={id} className="g7-h4">{title}</h4>
      {children}
    </section>
  )
}

const num = (text: string): number | undefined => (text.trim() === '' ? undefined : Number(text))

export function CostSection() {
  const id = useId()
  const { mutate } = useSWRConfig()
  const providers = useProviders()
  const catalog = useCatalog(true)
  const { data: appState } = useSWR('state', () => api.getState())
  const models = useMemo(() => sortCatalog(catalog.data?.models ?? [], 'name'), [catalog.data])
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null)
  const settings = providers.data?.settings ?? {}
  const currentModel = appState?.model ?? ''

  const save = async (patch: G9SettingsPatch, ok?: string) => {
    setMessage(null)
    try {
      await api.g9.saveSettings(patch)
      await Promise.all([mutate(PROVIDERS_KEY), mutate(CATALOG_KEY)])
      if (ok) setMessage({ ok: true, text: ok })
    } catch (err) {
      setMessage({ ok: false, text: errorMessage(err) })
    }
  }

  // ---- maliyet tavanı ----
  const [cap, setCap] = useState('')
  const saveCap = (e: FormEvent) => {
    e.preventDefault()
    const value = num(cap)
    if (value === undefined || !(value > 0)) {
      setMessage({ ok: false, text: 'Tavan sıfırdan büyük bir tutar olmalı.' })
      return
    }
    void save({ costCapUsd: value }, `Görev maliyet tavanı ${formatCost(value)} olarak ayarlandı.`).then(() => setCap(''))
  }

  // ---- en ucuz yeterli model ----
  const [needs, setNeeds] = useState<CapabilityNeeds>({ tools: true, vision: false, minContext: 0 })
  const usable = useMemo(() => new Set((providers.data?.providers ?? []).filter((p) => p.usable).map((p) => p.id)), [providers.data])
  const cheapest = useMemo(() => cheapestCapable(models, needs, (p) => usable.has(p)), [models, needs, usable])
  const pickModel = async (modelId: string) => {
    try {
      await api.setModel(modelId)
      await mutate('state')
      setMessage({ ok: true, text: `Ana model: ${modelId}` })
    } catch (err) {
      setMessage({ ok: false, text: errorMessage(err) })
    }
  }

  // ---- ön ayarlar ----
  const presets = settings.presets ?? {}
  const modelPresets = settings.modelPresets ?? {}
  const [preset, setPreset] = useState({ name: '', temperature: '', top_p: '', maxTokens: '' })
  const addPreset = (e: FormEvent) => {
    e.preventDefault()
    const value: ParamPreset = {}
    const t = num(preset.temperature)
    const p = num(preset.top_p)
    const m = num(preset.maxTokens)
    if (t !== undefined) value.temperature = t
    if (p !== undefined) value.top_p = p
    if (m !== undefined) value.maxTokens = m
    void save({ presets: { ...presets, [preset.name.trim()]: value } }, `"${preset.name.trim()}" ön ayarı kaydedildi.`).then(() =>
      setPreset({ name: '', temperature: '', top_p: '', maxTokens: '' }),
    )
  }
  const removePreset = (name: string) => {
    const nextPresets = Object.fromEntries(Object.entries(presets).filter(([n]) => n !== name))
    const nextAssign = Object.fromEntries(Object.entries(modelPresets).filter(([, n]) => n !== name))
    void save({ presets: nextPresets, modelPresets: nextAssign })
  }
  const assignPreset = (name: string) => {
    const next = { ...modelPresets }
    if (name) next[currentModel] = name
    else delete next[currentModel]
    void save({ modelPresets: next }, name ? `${currentModel} için "${name}" ön ayarı kullanılacak.` : `${currentModel} ön ayarsız.`)
  }

  // ---- fiyat tablosu ----
  const prices = settings.prices ?? {}
  const [row, setRow] = useState({ model: '', input: '', output: '', cacheInput: '', contextTokens: '' })
  const addPrice = (e: FormEvent) => {
    e.preventDefault()
    const input = num(row.input)
    const output = num(row.output)
    if (input === undefined || output === undefined) return
    const entry: PriceEntry = { input, output }
    const cacheInput = num(row.cacheInput)
    const contextTokens = num(row.contextTokens)
    if (cacheInput !== undefined) entry.cacheInput = cacheInput
    if (contextTokens !== undefined) entry.contextTokens = contextTokens
    void save({ prices: { ...prices, [row.model.trim()]: entry } }, `${row.model.trim()} fiyatı kaydedildi.`).then(() =>
      setRow({ model: '', input: '', output: '', cacheInput: '', contextTokens: '' }),
    )
  }

  // ---- yanıt önbelleği ----
  const cache = settings.responseCache ?? { enabled: false, ttlHours: 24, exclude: [] }
  const [exclude, setExclude] = useState<string | null>(null)

  if (providers.error) return <p className="error-text">{errorMessage(providers.error)}</p>
  if (!providers.data) return <p className="muted">Yükleniyor...</p>

  return (
    <div className="g7-stack">
      {message && (
        <p className={message.ok ? 'g7-note' : 'error-text'} role="status">
          {message.text}
        </p>
      )}

      <Sub title="Görev başına maliyet tavanı">
        <p className="g7-note text-pretty">
          Bir görev (tek mesajın turu ve alt ajanları) bu tutara ulaşınca ajan durur; akış sürerken tahmini tutar aşılırsa akış kesilir. Fiyatı bilinmeyen
          modeller hesaba katılamaz. Şu an: {settings.costCapUsd ? formatCost(settings.costCapUsd) : 'tavan yok'}.
        </p>
        <form className="g9-add-key" onSubmit={saveCap}>
          <label htmlFor={`${id}-cap`} className="sr-only">Tavan (USD)</label>
          <input id={`${id}-cap`} className="text-input" type="number" min={0.001} step={0.001} value={cap} onChange={(e) => setCap(e.target.value)} placeholder="ör. 0.50" />
          <button type="submit" className="btn small">Kaydet</button>
          {settings.costCapUsd != null && (
            <button type="button" className="btn small" onClick={() => void save({ costCapUsd: null }, 'Tavan kaldırıldı.')}>
              Kaldır
            </button>
          )}
        </form>
      </Sub>

      <Sub title="Düşük maliyet modu">
        <label className="g9-check">
          <input type="checkbox" checked={settings.lowCostMode === true} onChange={(e) => void save({ lowCostMode: e.target.checked })} />
          Başlık üretimi ve bağlam özetleme gibi yan işleri, Modeller bölümünde ayrıca seçilmemişse fiyatı bilinen en ucuz yeterli modele yönlendir
        </label>
      </Sub>

      <Sub title="En ucuz yeterli model">
        <div className="g7-inline">
          <label className="g9-check">
            <input type="checkbox" checked={needs.tools === true} onChange={(e) => setNeeds((n) => ({ ...n, tools: e.target.checked }))} /> Araç çağırma
          </label>
          <label className="g9-check">
            <input type="checkbox" checked={needs.vision === true} onChange={(e) => setNeeds((n) => ({ ...n, vision: e.target.checked }))} /> Görsel girdi
          </label>
          <label htmlFor={`${id}-ctx`}>En az bağlam</label>
          <select id={`${id}-ctx`} className="g7-select g9-narrow" value={needs.minContext ?? 0} onChange={(e) => setNeeds((n) => ({ ...n, minContext: Number(e.target.value) }))}>
            {CONTEXT_STEPS.map((c) => (
              <option key={c} value={c}>
                {c === 0 ? 'fark etmez' : formatTokens(c)}
              </option>
            ))}
          </select>
        </div>
        {cheapest ? (
          <div className="g7-inline">
            <span className="g9-profile-name">{cheapest.name}</span>
            <ProviderBadge model={cheapest} />
            <span className="g7-note">
              {cheapest.price ? `${formatUsd(cheapest.price.input)} / ${formatUsd(cheapest.price.output)} (1M girdi/çıktı)` : ''}
              {cheapest.contextTokens ? ` · ${formatTokens(cheapest.contextTokens)}` : ''}
            </span>
            <button type="button" className="btn small" onClick={() => void pickModel(cheapest.id)} disabled={cheapest.id === currentModel}>
              <Zap size={14} aria-hidden="true" /> {cheapest.id === currentModel ? 'Zaten seçili' : 'Ana model yap'}
            </button>
          </div>
        ) : (
          <p className="g7-note">Bu yeteneklere sahip, fiyatı bilinen model yok (model seçiciyi açıp listeyi yükle veya fiyat tablosuna ekle).</p>
        )}
      </Sub>

      <Sub title="Model parametre ön ayarları">
        {Object.keys(presets).length > 0 && (
          <ul className="g9-profiles" aria-label="Ön ayarlar">
            {Object.entries(presets).map(([name, p]) => (
              <li key={name} className="g9-profile">
                <span className="g9-profile-name">{name}</span>
                <span className="g7-note">
                  {[p.temperature !== undefined && `sıcaklık ${p.temperature}`, p.top_p !== undefined && `top_p ${p.top_p}`, p.maxTokens !== undefined && `maks ${p.maxTokens} token`]
                    .filter(Boolean)
                    .join(' · ') || 'boş'}
                </span>
                <span className="g9-grow" />
                <button type="button" className="action-btn" onClick={() => removePreset(name)} aria-label={`${name} ön ayarını sil`} title="Sil">
                  <Trash2 size={14} aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        )}
        <form className="g9-add-key" onSubmit={addPreset}>
          <label htmlFor={`${id}-pn`} className="sr-only">Ön ayar adı</label>
          <input id={`${id}-pn`} className="text-input" value={preset.name} onChange={(e) => setPreset({ ...preset, name: e.target.value })} placeholder="Ad (ör. Kesin)" maxLength={40} />
          <label htmlFor={`${id}-pt`} className="sr-only">Sıcaklık</label>
          <input id={`${id}-pt`} className="text-input g9-num" type="number" min={0} max={2} step={0.05} value={preset.temperature} onChange={(e) => setPreset({ ...preset, temperature: e.target.value })} placeholder="sıcaklık" />
          <label htmlFor={`${id}-pp`} className="sr-only">top_p</label>
          <input id={`${id}-pp`} className="text-input g9-num" type="number" min={0} max={1} step={0.05} value={preset.top_p} onChange={(e) => setPreset({ ...preset, top_p: e.target.value })} placeholder="top_p" />
          <label htmlFor={`${id}-pm`} className="sr-only">Maks token</label>
          <input id={`${id}-pm`} className="text-input g9-num" type="number" min={1} step={1} value={preset.maxTokens} onChange={(e) => setPreset({ ...preset, maxTokens: e.target.value })} placeholder="maks token" />
          <button type="submit" className="btn small" disabled={!preset.name.trim()}>Kaydet</button>
        </form>
        {currentModel && (
          <div className="g7-inline">
            <label htmlFor={`${id}-assign`}>
              Seçili model (<span className="g7-mono">{currentModel}</span>) için ön ayar
            </label>
            <select id={`${id}-assign`} className="g7-select g9-narrow" value={modelPresets[currentModel] ?? ''} onChange={(e) => assignPreset(e.target.value)}>
              <option value="">Yok (model varsayılanı)</option>
              {Object.keys(presets).map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </div>
        )}
      </Sub>

      <Sub title="Fiyat tablosu">
        <p className="g7-note text-pretty">
          USD / 1M token. Buradaki satır sağlayıcının bildirdiği fiyatı ve bağlam boyutunu ezer; fiyat vermeyen sağlayıcılar (OpenAI, Anthropic, Groq, Ollama) için
          maliyet tahmini, tavan ve en ucuz model seçimi bu tabloyla çalışır. Bağlam boyutu sıkıştırma eşiğini de belirler.
        </p>
        {Object.keys(prices).length > 0 && (
          <ul className="g9-profiles" aria-label="Fiyat satırları">
            {Object.entries(prices).map(([model, p]) => (
              <li key={model} className="g9-profile">
                <span className="g9-profile-name g7-mono">{model}</span>
                <span className="g7-note">
                  {formatUsd(p.input)} / {formatUsd(p.output)}
                  {p.cacheInput !== undefined ? ` · önbellek ${formatUsd(p.cacheInput)}` : ''}
                  {p.contextTokens ? ` · ${formatTokens(p.contextTokens)}` : ''}
                </span>
                <span className="g9-grow" />
                <button
                  type="button"
                  className="action-btn"
                  onClick={() => void save({ prices: Object.fromEntries(Object.entries(prices).filter(([m]) => m !== model)) })}
                  aria-label={`${model} fiyat satırını sil`}
                  title="Sil"
                >
                  <Trash2 size={14} aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        )}
        <form className="g9-add-key" onSubmit={addPrice}>
          <label htmlFor={`${id}-pmodel`} className="sr-only">Model kimliği</label>
          <input id={`${id}-pmodel`} className="text-input" list={`${id}-models`} value={row.model} onChange={(e) => setRow({ ...row, model: e.target.value })} placeholder="Model kimliği" spellCheck={false} />
          <datalist id={`${id}-models`}>
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name}
              </option>
            ))}
          </datalist>
          <label htmlFor={`${id}-pin`} className="sr-only">Girdi fiyatı</label>
          <input id={`${id}-pin`} className="text-input g9-num" type="number" min={0} step="any" value={row.input} onChange={(e) => setRow({ ...row, input: e.target.value })} placeholder="girdi" />
          <label htmlFor={`${id}-pout`} className="sr-only">Çıktı fiyatı</label>
          <input id={`${id}-pout`} className="text-input g9-num" type="number" min={0} step="any" value={row.output} onChange={(e) => setRow({ ...row, output: e.target.value })} placeholder="çıktı" />
          <label htmlFor={`${id}-pcache`} className="sr-only">Önbellek girdi fiyatı</label>
          <input id={`${id}-pcache`} className="text-input g9-num" type="number" min={0} step="any" value={row.cacheInput} onChange={(e) => setRow({ ...row, cacheInput: e.target.value })} placeholder="önbellek" />
          <label htmlFor={`${id}-pctx`} className="sr-only">Bağlam boyutu</label>
          <input id={`${id}-pctx`} className="text-input g9-num" type="number" min={1} step={1} value={row.contextTokens} onChange={(e) => setRow({ ...row, contextTokens: e.target.value })} placeholder="bağlam" />
          <button type="submit" className="btn small" disabled={!row.model.trim() || !row.input.trim() || !row.output.trim()}>
            Ekle
          </button>
        </form>
      </Sub>

      <Sub title="Yerel yanıt önbelleği">
        <p className="g7-note text-pretty">
          Varsayılan kapalı. Açıkken aynı model, araçlar ve (boşluk/büyük-küçük harf farkı dışında) aynı konuşmayla gelen istek, yanıtı yerelden alır ve ücret
          doğurmaz. Yalnızca bu makinede saklanır; sır içeren veya görsel/ek içeren istemler hiç önbelleğe girmez. Önbellekten gelen araç çağrıları yine izin
          denetiminden geçer. Kayıt sayısı: {providers.data.responseCache.entries}.
        </p>
        <label className="g9-check">
          <input type="checkbox" checked={cache.enabled} onChange={(e) => void save({ responseCache: { ...cache, enabled: e.target.checked } })} /> Önbelleği kullan
        </label>
        <div className="g7-inline">
          <label htmlFor={`${id}-ttl`}>Geçerlilik (saat)</label>
          <input
            id={`${id}-ttl`}
            className="text-input g9-num"
            type="number"
            min={1}
            max={720}
            defaultValue={cache.ttlHours}
            onBlur={(e) => {
              const v = Math.round(Number(e.target.value))
              if (v >= 1 && v <= 720 && v !== cache.ttlHours) void save({ responseCache: { ...cache, ttlHours: v } })
            }}
          />
          <button type="button" className="btn small" onClick={() => void api.g9.clearResponseCache().then(() => mutate(PROVIDERS_KEY), (err: unknown) => setMessage({ ok: false, text: errorMessage(err) }))}>
            Önbelleği temizle
          </button>
        </div>
        <label htmlFor={`${id}-excl`}>Hariç tutulacak ifadeler (satır başına bir; bunları içeren istem önbelleğe alınmaz)</label>
        <textarea
          id={`${id}-excl`}
          className="text-input g9-textarea"
          rows={3}
          value={exclude ?? cache.exclude.join('\n')}
          onChange={(e) => setExclude(e.target.value)}
          onBlur={() => {
            if (exclude === null) return
            const list = exclude.split('\n').map((x) => x.trim()).filter((x) => x.length >= 2)
            void save({ responseCache: { ...cache, exclude: list } }).then(() => setExclude(null))
          }}
        />
      </Sub>

      <Sub title="Model yetenek matrisi">
        <CapabilityMatrix models={models} />
      </Sub>
    </div>
  )
}
