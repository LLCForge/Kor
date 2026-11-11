// [budget] ve [routing] ayarları (§5.1). Okuma `cost.settings`, yazma `kor config set` karşılığı `core.configSet`
// (kullanıcı ayar dosyasına, doğrulanarak). Etkisi yeni turlarda.
import { useId, useState, type FormEvent } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { BudgetConfig, OnLimit, RouteCategory, RoutingConfig } from '../../../ipc-cost.ts'
import { api, errorMessage } from '../../api.ts'
import { useCatalog } from '../../g7-bus.ts'
import { costApi, keyPrefix } from './api.ts'
import { decimalSetting, percentSetting, ROUTE_CATEGORIES, validModelRef } from './shape.ts'

const ON_LIMIT: ReadonlyArray<{ value: OnLimit; label: string }> = [
  { value: 'ask', label: 'Sor' },
  { value: 'downgrade', label: 'En ucuz bilinen modele geç' },
  { value: 'pause', label: 'Durdur' },
]

const isCostKey = keyPrefix('cost:')

export function BudgetSettings() {
  const settings = useSWR(['cost:settings'], () => costApi.settings(), { revalidateOnFocus: false })
  if (!settings.data) {
    return settings.error ? <p className="error-text" role="alert">{errorMessage(settings.error)}</p> : <p className="muted">Yükleniyor...</p>
  }
  return <BudgetForm key={JSON.stringify(settings.data)} budget={settings.data.budget} routing={settings.data.routing} />
}

type RouteKey = RouteCategory | 'verify_with'

function BudgetForm({ budget, routing }: { budget: BudgetConfig; routing: RoutingConfig }) {
  const id = useId()
  const { mutate } = useSWRConfig()
  const catalog = useCatalog(true)
  const [b, setB] = useState({
    daily: String(budget.daily_limit_usd),
    monthly: String(budget.monthly_limit_usd),
    alert: String(budget.alert_at_percent),
    onLimit: budget.on_limit,
  })
  const [enabled, setEnabled] = useState(routing.enabled)
  const [routes, setRoutes] = useState<Record<RouteKey, string>>(() => ({
    ...Object.fromEntries(ROUTE_CATEGORIES.map((c) => [c.key, routing[c.key]])),
    verify_with: routing.verify_with,
  }) as Record<RouteKey, string>)
  const [consensus, setConsensus] = useState(routing.consensus.join(', '))
  const [shadow, setShadow] = useState({ model: routing.shadow, threshold: String(routing.shadow_threshold || 85), samples: String(routing.shadow_min_samples || 5) })
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null)

  const save = async (e: FormEvent) => {
    e.preventDefault()
    setMessage(null)
    const daily = decimalSetting(b.daily)
    const monthly = decimalSetting(b.monthly)
    const alert = percentSetting(b.alert)
    if (daily === null || monthly === null) return setMessage({ ok: false, text: 'Limitler sıfır ya da pozitif bir tutar olmalı (0 = limit yok).' })
    if (alert === null) return setMessage({ ok: false, text: 'Uyarı eşiği 0–100 arası bir tamsayı olmalı.' })
    const bad = Object.entries(routes).find(([, v]) => !validModelRef(v))
    if (bad) return setMessage({ ok: false, text: `Geçersiz model: ${bad[1]} (sağlayıcı/model biçiminde olmalı ya da boş kalmalı)` })
    const consensusList = consensus.split(',').map((m) => m.trim()).filter(Boolean)
    const badConsensus = consensusList.find((m) => !validModelRef(m))
    if (badConsensus || consensusList.length === 1 || consensusList.length > 3) return setMessage({ ok: false, text: 'Consensus için 2–3 model yazın (sağlayıcı/model, virgülle) ya da boş bırakın.' })
    if (!validModelRef(shadow.model)) return setMessage({ ok: false, text: `Geçersiz gölge model: ${shadow.model}` })
    const threshold = percentSetting(shadow.threshold)
    const samples = Number(shadow.samples)
    if (threshold === null || !Number.isInteger(samples) || samples < 1 || samples > 100) return setMessage({ ok: false, text: 'Gölge eşiği 0–100, örnek sayısı 1–100 olmalı.' })

    // Yalnızca değişen anahtarlar yazılır; her biri yazılmadan önce çekirdekte doğrulanır.
    const changes: Array<[string, string]> = []
    if (Number(daily) !== budget.daily_limit_usd) changes.push(['budget.daily_limit_usd', daily])
    if (Number(monthly) !== budget.monthly_limit_usd) changes.push(['budget.monthly_limit_usd', monthly])
    if (Number(alert) !== budget.alert_at_percent) changes.push(['budget.alert_at_percent', alert])
    if (b.onLimit !== budget.on_limit) changes.push(['budget.on_limit', b.onLimit])
    if (enabled !== routing.enabled) changes.push(['routing.enabled', String(enabled)])
    for (const [k, v] of Object.entries(routes) as Array<[RouteKey, string]>) {
      if (v.trim() !== (k === 'verify_with' ? routing.verify_with : routing[k])) changes.push([`routing.${k}`, v.trim()])
    }
    if (consensusList.join(',') !== routing.consensus.join(',')) changes.push(['routing.consensus', JSON.stringify(consensusList)])
    if (shadow.model.trim() !== routing.shadow) changes.push(['routing.shadow', shadow.model.trim()])
    if (Number(threshold) !== routing.shadow_threshold) changes.push(['routing.shadow_threshold', threshold])
    if (samples !== routing.shadow_min_samples) changes.push(['routing.shadow_min_samples', String(samples)])
    if (changes.length === 0) return setMessage({ ok: true, text: 'Değişiklik yok.' })

    setBusy(true)
    try {
      let explain = null
      for (const [key, value] of changes) explain = await api.core.configSet(key, value)
      const shadowed = (explain?.settings ?? []).filter((s) => changes.some(([k]) => k === s.key) && (s.layer === 'project' || s.layer === 'env' || s.layer === 'cli'))
      setMessage(
        shadowed.length > 0
          ? { ok: false, text: `Kaydedildi, ama şu ayarları proje/ortam katmanı eziyor: ${shadowed.map((s) => `${s.key} (${s.origin})`).join(', ')}` }
          : { ok: true, text: 'Kaydedildi. Yeni turlarda geçerli.' },
      )
      await mutate(isCostKey)
    } catch (err) {
      setMessage({ ok: false, text: errorMessage(err) })
    } finally {
      setBusy(false)
    }
  }

  return (
    <form className="g2c-stack" onSubmit={(e) => void save(e)} data-kor-cmd="core.configSet">
      <section className="g14-section" aria-labelledby={`${id}-budget`} data-kor-cmd="cost.settings">
        <h3 id={`${id}-budget`} className="g14-h">Bütçe</h3>
        <p className="g7-note text-pretty">0 = limit yok. Gün ve ay yerel saate göre sayılır. Limitler maliyet panosunda ve her turda denetlenir.</p>
        <div className="g2c-form">
          <label htmlFor={`${id}-daily`}>Günlük limit (USD)</label>
          <input id={`${id}-daily`} className="text-input g9-num" type="number" min={0} step="any" value={b.daily} onChange={(e) => setB({ ...b, daily: e.target.value })} />
          <label htmlFor={`${id}-monthly`}>Aylık limit (USD)</label>
          <input id={`${id}-monthly`} className="text-input g9-num" type="number" min={0} step="any" value={b.monthly} onChange={(e) => setB({ ...b, monthly: e.target.value })} />
          <label htmlFor={`${id}-alert`}>Uyarı eşiği (%)</label>
          <input id={`${id}-alert`} className="text-input g9-num" type="number" min={0} max={100} step={1} value={b.alert} onChange={(e) => setB({ ...b, alert: e.target.value })} />
          <label htmlFor={`${id}-onlimit`}>Limite ulaşınca</label>
          <select id={`${id}-onlimit`} className="g7-select" value={b.onLimit} onChange={(e) => setB({ ...b, onLimit: e.target.value as OnLimit })}>
            {ON_LIMIT.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </div>
      </section>

      <section className="g14-section" aria-labelledby={`${id}-routing`}>
        <h3 id={`${id}-routing`} className="g14-h">Akıllı yönlendirme</h3>
        <label className="g9-check">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} /> Her görevi sınıflandır ve kategorisinin modeline gönder
        </label>
        <p className="g7-note text-pretty">Boş kategori: katalogdan yeterli en ucuz bilinen model seçilir. Biçim: sağlayıcı/model.</p>
        <div className="g2c-form">
          {ROUTE_CATEGORIES.map((c) => (
            <RouteInput key={c.key} id={`${id}-${c.key}`} label={c.label} value={routes[c.key]} list={`${id}-models`} onChange={(v) => setRoutes({ ...routes, [c.key]: v })} />
          ))}
          <RouteInput id={`${id}-verify`} label="Taslak sonrası inceleyen model" value={routes.verify_with} list={`${id}-models`} onChange={(v) => setRoutes({ ...routes, verify_with: v })} />
        </div>
        <datalist id={`${id}-models`}>
          {(catalog.data?.models ?? []).map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </datalist>
        <h4 className="g14-h">Consensus ve gölge modu</h4>
        <p className="g7-note text-pretty">Consensus: aynı soru bu 2–3 modele paralel sorulur (Maliyet sekmesindeki Consensus bölümü ya da komut satırında kor consensus).</p>
        <p className="g7-note text-pretty">Gölge: pahalı model bir görevi bitirince bu ucuz model arka planda aynı soruyu yanıtlar; ortalama uyum eşiği geçince ucuz modele geçmeniz önerilir.</p>
        <div className="g2c-form">
          <label htmlFor={`${id}-consensus`}>Consensus modelleri</label>
          <input id={`${id}-consensus`} className="text-input" list={`${id}-models`} value={consensus} onChange={(e) => setConsensus(e.target.value)} placeholder="sağlayıcı/model, sağlayıcı/model" spellCheck={false} maxLength={600} translate="no" />
          <RouteInput id={`${id}-shadow`} label="Gölge model" value={shadow.model} list={`${id}-models`} onChange={(v) => setShadow({ ...shadow, model: v })} />
          <label htmlFor={`${id}-sthr`}>Öneri eşiği (%)</label>
          <input id={`${id}-sthr`} className="text-input g9-num" type="number" min={0} max={100} step={1} value={shadow.threshold} onChange={(e) => setShadow({ ...shadow, threshold: e.target.value })} />
          <label htmlFor={`${id}-ssmp`}>En az görev</label>
          <input id={`${id}-ssmp`} className="text-input g9-num" type="number" min={1} max={100} step={1} value={shadow.samples} onChange={(e) => setShadow({ ...shadow, samples: e.target.value })} />
        </div>
        {routing.fallback.length > 0 && (
          <p className="g7-note">
            Yedek modeller (config.toml'dan): <span className="g7-mono" translate="no">{routing.fallback.join(', ')}</span>
          </p>
        )}
      </section>

      <div className="g7-inline">
        <button type="submit" className="btn primary small" disabled={busy}>
          {busy ? 'Kaydediliyor...' : 'Kaydet'}
        </button>
        {message && (
          <p className={message.ok ? 'g7-note' : 'error-text'} role="status">
            {message.text}
          </p>
        )}
      </div>
    </form>
  )
}

function RouteInput({ id, label, value, list, onChange }: { id: string; label: string; value: string; list: string; onChange: (v: string) => void }) {
  return (
    <>
      <label htmlFor={id}>{label}</label>
      <input id={id} className="text-input" list={list} value={value} onChange={(e) => onChange(e.target.value)} placeholder="katalogdan" spellCheck={false} maxLength={200} translate="no" />
    </>
  )
}
