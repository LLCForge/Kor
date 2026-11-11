// [g27] Ayarlar: kullanım çubukları, yetenekler, bellek, tasarım listesi ve algılanan araçlar.
import { useEffect, useId, useState, type FormEvent } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { InstallPhase } from '../../core/runtimes-g27.ts'
import type { PermissionMode } from '../../core/permissions.ts'
import { api, errorMessage } from './api.ts'
import { MODES } from './ModeMenu.tsx'
import { uiLocale, percentText } from './ui-locale.ts'

interface RuntimeRow {
  id: string
  name: string
  installed: boolean
  version: string | null
}

const DESIGN_KEY = 'kor-design-systems'

interface DesignRow {
  name: string
  updated: string
}

function loadDesigns(): DesignRow[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(DESIGN_KEY) ?? '[]') as unknown
    if (!Array.isArray(parsed)) return []
    return parsed.filter((row): row is DesignRow => typeof row === 'object' && row !== null && 'name' in row && typeof row.name === 'string')
  } catch {
    return []
  }
}

export function UsageSettings() {
  const usage = useSWR('g27:usage', () => api.getUsage(null), { revalidateOnFocus: false })
  const live = usage.data?.live?.tokens
  const pct = live && live.limit > 0 ? Math.min(100, Math.round(((live.limit - live.remaining) / live.limit) * 100)) : 0
  const session = usage.data?.session
  const sessionTokens = session ? session.promptTokens + session.completionTokens : 0
  return (
    <div className="g7-stack">
      <p className="g7-note">Dakikalık çubuk son yanıttaki gerçek sınıra göredir. Oturum satırı kota yüzdesi değildir; yalnız token sayısını gösterir.</p>
      <UsageBar label="Mevcut oturum" detail={session ? `${sessionTokens} token` : 'Henüz ölçüm yok'} pct={0} showPercent={false} />
      <UsageBar label="Dakikalık token sınırı" detail={live && live.limit > 0 ? `Kalan ${live.remaining}` : 'Henüz ölçüm yok'} pct={live && live.limit > 0 ? pct : 0} showPercent={Boolean(live && live.limit > 0)} />
      {usage.data?.balances && <p className="g7-note">Bakiye: {usage.data.balances.usd.toLocaleString(uiLocale(), { style: 'currency', currency: 'USD' })}</p>}
      {usage.error && <p className="error-text">{errorMessage(usage.error)}</p>}
    </div>
  )
}

function UsageBar({ label, detail, pct, showPercent = true }: { label: string; detail: string; pct: number; showPercent?: boolean }) {
  return (
    <div className="g27-usage">
      <div className="g27-usage-head">
        <span>{label}</span>
        <span>{showPercent ? percentText(pct) : detail}</span>
      </div>
      {showPercent && <p className="g7-note">{detail}</p>}
      <div className="g27-bar" role="progressbar" aria-valuenow={showPercent ? pct : undefined} aria-valuemin={0} aria-valuemax={100} aria-label={label}>
        {showPercent && <span style={{ width: `${pct}%` }} />}
      </div>
    </div>
  )
}

export function CapabilitiesSettings() {
  const state = useSWR('state', () => api.getState())
  const { mutate } = useSWRConfig()
  const mode = state.data?.permissionMode ?? 'default'
  const setMode = async (next: PermissionMode) => {
    await api.setPermissionMode(next)
    await mutate('state')
  }
  return (
    <div className="g7-stack">
      <div className="g7-setting">
        <div className="g7-setting-text">
          <span className="g7-setting-title">Araç erişim modu</span>
          <span className="g7-note">Düzenleme ve komutların ne zaman onay istediğini belirler.</span>
        </div>
        <select className="text-input" value={mode} onChange={(e) => void setMode(e.target.value as PermissionMode)} aria-label="Araç erişim modu">
          {MODES.map((item) => (
            <option key={item.id} value={item.id}>{item.label}</option>
          ))}
        </select>
      </div>
      <p className="g7-note">Sohbet içi grafik, diyagram ve dosya araçları bu modun izin verdiği ölçüde açıktır. Kod çalıştırma da aynı onay kapısından geçer.</p>
      <ContextCompactionSettings />
    </div>
  )
}

/**
 * [g6] Otomatik bağlam sıkıştırma eşiği. Uzun bağlamlı modellerde sabit %70 eşiği çok geç kalıyordu; tavan token
 * değeri burada ayarlanır. Boş bırakılırsa varsayılan (48k / %70) kullanılır.
 */
function ContextCompactionSettings() {
  const { data, mutate } = useSWR('g6:compactSettings', () => api.getCompactSettings())
  const [maxTokens, setMaxTokens] = useState('')
  const [ratio, setRatio] = useState('')
  const [saved, setSaved] = useState(false)
  useEffect(() => {
    if (!data) return
    setMaxTokens(data.maxTokens === null ? '' : String(data.maxTokens))
    setRatio(data.ratio === null ? '' : String(Math.round(data.ratio * 100)))
  }, [data])
  const save = async () => {
    const tokens = maxTokens.trim() === '' ? null : Number(maxTokens)
    const pct = ratio.trim() === '' ? null : Number(ratio) / 100
    await api.setCompactSettings({ maxTokens: Number.isFinite(tokens as number) ? tokens : null, ratio: pct !== null && Number.isFinite(pct) ? pct : null })
    await mutate()
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }
  return (
    <>
      <div className="g7-setting">
        <div className="g7-setting-text">
          <span className="g7-setting-title">Otomatik sıkıştırma eşiği (token)</span>
          <span className="g7-note">Bağlam bu token sayısına ulaşınca eski turlar özetlenir. Boş: model sınırının %70'i, en çok 48.000.</span>
        </div>
        <div className="g7-setting-control">
          <input className="text-input g6-num" inputMode="numeric" value={maxTokens} placeholder="48000" onChange={(e) => setMaxTokens(e.target.value)} aria-label="Sıkıştırma eşiği token" />
          <span className="g7-note">token</span>
        </div>
      </div>
      <div className="g7-setting">
        <div className="g7-setting-text">
          <span className="g7-setting-title">Eşik oranı (%)</span>
          <span className="g7-note">Model penceresine göre yüzde. Tavan değerinden küçük olan geçerlidir. Boş: %70.</span>
        </div>
        <div className="g7-setting-control">
          <input className="text-input g6-num" inputMode="numeric" value={ratio} placeholder="70" onChange={(e) => setRatio(e.target.value)} aria-label="Sıkıştırma oranı yüzde" />
          <span className="g7-note">%</span>
        </div>
      </div>
      <div className="g7-setting">
        <span className="g7-note">{saved ? 'Kaydedildi.' : 'Değişiklik yalnızca bu bilgisayardaki kullanıcı ayarına yazılır.'}</span>
        <button type="button" className="btn small" onClick={() => void save()}>Kaydet</button>
      </div>
    </>
  )
}

export function MemorySettings({ onOpen }: { onOpen: () => void }) {
  return (
    <div className="g7-stack">
      <div className="g7-setting">
        <div className="g7-setting-text">
          <span className="g7-setting-title">Sohbetlerde arama ve bellek</span>
          <span className="g7-note">Ajanın tüm sohbetlerde hatırladığı notlar bu bilgisayardadır.</span>
        </div>
        <button type="button" className="btn small" onClick={onOpen}>Düzenle</button>
      </div>
    </div>
  )
}

export function DesignSettings() {
  const [rows, setRows] = useState<DesignRow[]>(() => loadDesigns())
  const [name, setName] = useState('')
  const add = (e: FormEvent) => {
    e.preventDefault()
    const trimmed = name.trim().slice(0, 80)
    if (!trimmed) return
    const next = [{ name: trimmed, updated: new Date().toLocaleDateString(uiLocale()) }, ...rows.filter((row) => row.name !== trimmed)]
    localStorage.setItem(DESIGN_KEY, JSON.stringify(next))
    setRows(next)
    setName('')
  }
  return (
    <div className="g7-stack">
      <p className="g7-note">Renk ve yazı tipi adları bu bilgisayarda durur. Uzak bir mağaza yoktur.</p>
      <form className="g7-inline" onSubmit={add}>
        <input className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="Ada göre ekle" aria-label="Tasarım sistemi adı" />
        <button type="submit" className="btn small" disabled={!name.trim()}>Ekle</button>
      </form>
      {rows.length === 0 && <p className="muted">Kayıtlı tasarım sistemi yok.</p>}
      {rows.map((row) => (
        <div key={row.name} className="g7-setting">
          <span className="g7-setting-title">{row.name}</span>
          <span className="g7-note">{row.updated}</span>
        </div>
      ))}
    </div>
  )
}

export function SystemSettings() {
  const settings = useSWR('g7:settings', () => api.g7GetSettings())
  const app = settings.data?.app
  return (
    <div className="g7-stack">
      <div className="g7-setting">
        <span className="g7-setting-title">Masaüstü uygulaması sürümü</span>
        <span>{app?.version ?? '—'}</span>
      </div>
      <div className="g7-setting">
        <span className="g7-setting-title">Tauri / {app?.webviewName ?? 'WebView'}</span>
        <span className="g7-note">{app ? `${app.tauri} / ${app.webview || '—'}` : '—'}</span>
      </div>
    </div>
  )
}

const PHASE: Record<InstallPhase, string> = {
  indiriliyor: 'İndiriliyor',
  kuruluyor: 'Kuruluyor',
  dogrulaniyor: 'Doğrulanıyor',
  bitti: 'Bitti',
  hata: 'Hata',
}

export function RuntimesPanel() {
  const id = useId()
  const list = useSWR('g27:runtimes', () => api.g27ListRuntimes())
  const [progress, setProgress] = useState<Record<string, { phase: InstallPhase; percent: number | null; line: string }>>({})
  const [error, setError] = useState<string | null>(null)

  useEffect(() => api.onRuntimeProgress((tick) => {
    setProgress((prev) => ({ ...prev, [tick.id]: { phase: tick.phase, percent: tick.percent, line: tick.line } }))
  }), [])

  const install = async (row: RuntimeRow) => {
    setError(null)
    setProgress((prev) => ({ ...prev, [row.id]: { phase: 'indiriliyor', percent: null, line: 'Başlatıldı' } }))
    try {
      await api.g27InstallRuntime(row.id)
      await list.mutate()
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  if (list.error) return <p className="error-text">{errorMessage(list.error)}</p>
  if (!list.data) return <p className="muted">Araçlar taranıyor…</p>

  return (
    <div className="g7-stack">
      <h4 className="g7-h4" id={id}>Algılanan araçlar</h4>
      <p className="g7-note">Kurulu olanlar sürümüyle listelenir. Eksik olan yalnız Kur düğmesine basınca kurulur.</p>
      {list.data.map((row) => {
        const tick = progress[row.id]
        return (
          <div key={row.id} className="g27-runtime">
            <div className="g7-setting">
              <div className="g7-setting-text">
                <span className="g7-setting-title">{row.name}</span>
                <span className="g7-note">{row.installed ? row.version : 'Kurulu değil'}</span>
              </div>
              {row.installed ? <span className="g27-ok">Kurulu</span> : (
                <button type="button" className="btn small" disabled={tick?.phase === 'indiriliyor' || tick?.phase === 'kuruluyor' || tick?.phase === 'dogrulaniyor'} onClick={() => void install(row)}>
                  Kur
                </button>
              )}
            </div>
            {tick && !row.installed && (
              <div className="g27-install">
                <div className="g27-usage-head">
                  <span>{PHASE[tick.phase]}</span>
                  <span>{tick.percent === null ? 'ölçülüyor' : percentText(tick.percent)}</span>
                </div>
                <div className={`g27-bar${tick.percent === null && tick.phase !== 'hata' && tick.phase !== 'bitti' ? ' g27-bar-wait' : ''}`} role="progressbar" aria-valuenow={tick.percent ?? undefined} aria-valuemin={0} aria-valuemax={100} aria-label={`${row.name} kurulumu`}>
                  {tick.percent !== null && <span style={{ width: `${tick.percent}%` }} />}
                </div>
                <p className="g7-note">{tick.line}</p>
              </div>
            )}
          </div>
        )
      })}
      {error && <p className="error-text" role="alert">{error}</p>}
    </div>
  )
}
