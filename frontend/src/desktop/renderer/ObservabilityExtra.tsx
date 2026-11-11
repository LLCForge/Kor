// [g14] Gözlem penceresinin maliyet, süreç, ham istek ve ayar sekmeleri.
import { useEffect, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import { forecastSpend, type CostSlice, type SpendForecast } from '../../core/metrics-g14.ts'
import type { ObservabilitySettings } from '../ipc-g14.ts'
import { api, errorMessage } from './api.ts'
import { Columns, HBars, statusName } from './g14-charts.tsx'
import { formatClock, formatDayTime, formatMb, formatMs, formatPercent, formatTokens, formatUsd } from './g14-format.ts'
import { CopyButton } from './CopyButton.tsx'

const usdBars = (slices: readonly CostSlice[]) =>
  slices.map((s) => ({ key: s.key, value: s.usd, text: formatUsd(s.usd), title: `${s.key}: ${formatUsd(s.usd)}, ${formatTokens(s.tokens)} token, ${Math.round(s.calls * 10) / 10} çağrı` }))

function localDay(t: number): string {
  const d = new Date(t)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

// ---------- Maliyet ----------

export function CostTab({ sessionId, onModelChange }: { sessionId: string | null; onModelChange: (model: string) => void }) {
  const [days, setDays] = useState<7 | 30 | 90>(30)
  const { data, error } = useSWR(['g14:cost', days], () => api.g14.cost(days), { refreshInterval: 10_000, revalidateOnFocus: false })
  // Hesap geneli tahmin için Venice faturalama verisi (g7 uç noktası); alınamazsa yalnızca yerel tahmin gösterilir.
  const billing = useSWR('g14:billing', () => api.g7GetUsageHistory('day'), { revalidateOnFocus: false, shouldRetryOnError: false })
  const billingForecast = billing.data ? forecastSpend(billing.data.buckets.map((b) => ({ date: b.start, usd: b.usd })), new Date()) : null

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>
  const a = data.attribution
  const dayMap = new Map(a.byDay.map((d) => [d.key, d.usd]))
  const dayPoints = Array.from({ length: days }, (_, i) => {
    const key = localDay(Date.now() - (days - 1 - i) * 86_400_000)
    const usd = dayMap.get(key) ?? 0
    return { label: key.slice(5), value: usd, title: `${key}: ${formatUsd(usd)}` }
  })
  const rec = data.recommendation

  return (
    <>
      <section className="g14-section" aria-label="Maliyet özeti">
        <div className="g14-row">
          <div className="g14-periods" role="group" aria-label="Dönem">
            {([7, 30, 90] as const).map((d) => (
              <button key={d} type="button" className={`btn small${days === d ? ' active' : ''}`} aria-pressed={days === d} onClick={() => setDays(d)}>
                {d} gün
              </button>
            ))}
          </div>
          <p className="g14-total">
            Toplam <strong>{formatUsd(a.totalUsd)}</strong>
          </p>
          {a.unpricedCalls > 0 && <p className="g14-note">{a.unpricedCalls} çağrının fiyatı bilinmiyor (model listesi yüklenince hesaplanır).</p>}
        </div>
        <p className="g14-note">Tahmini maliyet: bu uygulamanın model çağrılarından, katalog fiyatlarıyla hesaplanır. Önbellek indirimi ve sıkıştırma/başlık çağrıları dahil değildir.</p>
        <Columns points={dayPoints} max={0} label={`Günlük harcama, son ${days} gün`} format={(v) => formatUsd(v)} />
      </section>

      <section className="g14-section" aria-label="Maliyet atfı">
        <h3 className="g14-h">Maliyet atfı</h3>
        <div className="g14-grid2">
          <div>
            <p className="g14-sub">Model</p>
            <HBars label="Modele göre maliyet" items={usdBars(a.byModel)} />
          </div>
          <div>
            <p className="g14-sub">Araç (yanıtın istediği araçlara eşit bölünür)</p>
            <HBars label="Araca göre maliyet" items={usdBars(a.byTool)} />
          </div>
          <div>
            <p className="g14-sub">Proje</p>
            <HBars label="Projeye göre maliyet" items={usdBars(a.byProject)} />
          </div>
          <div>
            <p className="g14-sub">Etiket (bir çağrı her etiketine tam sayılır)</p>
            <HBars label="Etikete göre maliyet" items={usdBars(a.byTag)} />
          </div>
        </div>
      </section>

      <section className="g14-section" aria-label="Harcama tahmini">
        <h3 className="g14-h">Harcama tahmini</h3>
        <div className="g14-cards">
          <ForecastCard title="Bu uygulama (yerel defter)" forecast={data.forecast} />
          {billingForecast && <ForecastCard title="Hesap geneli (faturalama, USD)" forecast={billingForecast} />}
        </div>
        {billing.error && <p className="g14-note">Faturalama verisi alınamadı; yalnızca yerel tahmin gösteriliyor.</p>}
      </section>

      <section className="g14-section" aria-label="Kalite ve maliyet">
        <h3 className="g14-h">Kalite/maliyet geri bildirimi ve model önerisi</h3>
        <p className="g14-note">Yanıtları mesaj kutusundaki veya İzler sekmesindeki beğen/beğenme düğmeleriyle değerlendirin. Öneri: en yüksek beğeni puanına yakın modeller arasında tur başına en ucuz olanı.</p>
        {rec.pick ? (
          <div className="g14-rec">
            <span className="g14-rec-label">Önerilen model</span>
            <strong>{rec.pick}</strong>
            <span className="muted">{rec.reason}</span>
            <button type="button" className="btn small primary" onClick={() => onModelChange(rec.pick ?? '')}>
              Bu modeli kullan
            </button>
          </div>
        ) : (
          <p className="muted">{rec.reason}</p>
        )}
        {rec.stats.length > 0 && (
          <div className="g7-table-wrap">
            <table className="g7-table">
              <thead>
                <tr>
                  <th scope="col">Model</th>
                  <th scope="col" className="num">Beğeni</th>
                  <th scope="col" className="num">Beğenmeme</th>
                  <th scope="col" className="num">Puan</th>
                  <th scope="col" className="num">Ort. tur maliyeti</th>
                </tr>
              </thead>
              <tbody>
                {rec.stats.map((s) => (
                  <tr key={s.model} className={s.model === rec.pick ? 'selected' : undefined}>
                    <th scope="row">{s.model}</th>
                    <td className="num">{s.likes}</td>
                    <td className="num">{s.dislikes}</td>
                    <td className="num">{formatPercent(s.score)}</td>
                    <td className="num">{formatUsd(s.avgTurnCostUsd)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <section className="g14-section" aria-label="Maliyet etiketleri">
        <h3 className="g14-h">Maliyet etiketleri</h3>
        {sessionId ? <TagsEditor sessionId={sessionId} /> : <p className="muted ks-empty">Etiket eklemek için bir sohbet açın.</p>}
      </section>
    </>
  )
}

function ForecastCard({ title, forecast }: { title: string; forecast: SpendForecast }) {
  return (
    <div className="g14-card">
      <span className="g14-card-title">{title}</span>
      <span className="g14-card-value">{formatUsd(forecast.monthProjectionUsd)} ay sonu</span>
      <dl className="g14-dl">
        <div>
          <dt>Ay başından beri</dt>
          <dd>{formatUsd(forecast.monthToDateUsd)}</dd>
        </div>
        <div>
          <dt>Gelecek 7 gün</dt>
          <dd>{formatUsd(forecast.nextWeekUsd)}</dd>
        </div>
        <div>
          <dt>Günlük ort. (7 / 30 gün)</dt>
          <dd>
            {formatUsd(forecast.avg7)} / {formatUsd(forecast.avg30)}
          </dd>
        </div>
      </dl>
      {forecast.sampleDays < 7 && <span className="g14-card-detail">Son 30 günde yalnızca {forecast.sampleDays} gün harcama var; tahmin kaba.</span>}
    </div>
  )
}

function TagsEditor({ sessionId }: { sessionId: string }) {
  const { data, mutate } = useSWR(['g14:tags', sessionId], () => api.g14.getTags(sessionId), { revalidateOnFocus: false })
  const [text, setText] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  useEffect(() => {
    if (data) setText(data.join(', '))
  }, [data])

  const save = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    setSaved(false)
    const tags = text
      .split(/[,\s]+/)
      .map((t) => t.trim())
      .filter(Boolean)
    try {
      await api.g14.setTags(sessionId, tags)
      await mutate()
      setSaved(true)
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  return (
    <form className="g14-form-row" onSubmit={(e) => void save(e)}>
      <label htmlFor="g14-tags">Bu sohbetin etiketleri</label>
      <input
        id="g14-tags"
        className="g14-input"
        value={text}
        maxLength={300}
        placeholder="ör. görev:refactor, müşteri:acme"
        onChange={(e) => {
          setText(e.target.value)
          setSaved(false)
        }}
        aria-describedby="g14-tags-help"
      />
      <button type="submit" className="btn small">
        Kaydet
      </button>
      <span id="g14-tags-help" className="g14-note">
        Virgülle ayırın; en fazla 8 etiket. Yeni model çağrılarına uygulanır.
      </span>
      {error && <span className="error-text" role="alert">{error}</span>}
      {saved && <span className="muted" role="status">Kaydedildi.</span>}
    </form>
  )
}

// ---------- Süreçler ----------

export function ProcessesTab() {
  const { data, error } = useSWR('g14:processes', () => api.g14.processes(), { refreshInterval: 5_000, revalidateOnFocus: false })
  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Süreçler ölçülüyor...</p>
  const matched = new Set(data.groups.flatMap((g) => (g.shellId ? [g.shellId] : [])))
  const orphanShells = data.shells.filter((s) => !matched.has(s.id))

  return (
    <>
      <section className="g14-section" aria-label="Arka plan kabukları ve alt süreçler">
        <h3 className="g14-h">Arka plan kabukları ve alt süreçler</h3>
        <p className="g14-note">CPU, iki ölçüm arasındaki farktan hesaplanır (5 sn'de bir); ilk ölçümde boş kalır. Alt süreçlerin tamamı (ör. npm altındaki node) gruba dahildir.</p>
        {data.error && <p className="error-text" role="alert">{data.error}</p>}
        {data.groups.length === 0 && !data.error ? (
          <p className="muted ks-empty">Çalışan alt süreç yok.</p>
        ) : (
          <div className="g7-table-wrap">
            <table className="g7-table">
              <thead>
                <tr>
                  <th scope="col">Süreç</th>
                  <th scope="col">Tür</th>
                  <th scope="col" className="num">PID</th>
                  <th scope="col" className="num">Alt süreç</th>
                  <th scope="col" className="num">CPU</th>
                  <th scope="col" className="num">Bellek</th>
                </tr>
              </thead>
              <tbody>
                {data.groups.map((g) => (
                  <tr key={g.rootPid}>
                    <th scope="row" className="g14-proc-label">
                      <code className="truncate" title={g.label}>{g.label}</code>
                    </th>
                    <td>{g.shellId ? `Arka plan kabuğu (${g.shellId})` : 'Alt süreç'}</td>
                    <td className="num">{g.rootPid}</td>
                    <td className="num">{g.pids.length - 1}</td>
                    <td className="num">{g.cpuPercent === null ? '-' : `${g.cpuPercent.toFixed(1)}%`}</td>
                    <td className="num">{formatMb(g.memBytes / 1_048_576)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
        {orphanShells.length > 0 && (
          <p className="g14-note">Süreci eşleştirilemeyen kabuklar: {orphanShells.map((s) => s.id).join(', ')}</p>
        )}
      </section>

      <section className="g14-section" aria-label="Uygulama süreçleri">
        <h3 className="g14-h">Uygulama süreçleri</h3>
        <div className="g7-table-wrap">
          <table className="g7-table">
            <thead>
              <tr>
                <th scope="col">Tür</th>
                <th scope="col" className="num">PID</th>
                <th scope="col" className="num">CPU</th>
                <th scope="col" className="num">Bellek</th>
              </tr>
            </thead>
            <tbody>
              {data.app.map((p) => (
                <tr key={p.pid}>
                  <th scope="row">{p.type === 'Browser' ? 'Ana süreç' : p.type === 'Tab' ? 'Arayüz' : p.type}</th>
                  <td className="num">{p.pid}</td>
                  <td className="num">{p.cpuPercent.toFixed(1)}%</td>
                  <td className="num">{formatMb(p.memMb)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <p className="g14-note">Son ölçüm {formatClock(data.at)}</p>
      </section>
    </>
  )
}

// ---------- Ham istekler ----------

export function RawTab() {
  const { data, error } = useSWR('g14:raw', () => api.g14.raw(), { refreshInterval: 5_000, revalidateOnFocus: false })
  const [selected, setSelected] = useState<string | null>(null)
  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>
  const item = data.find((r) => r.id === selected) ?? data[0]

  return (
    <div className="g14-split">
      <section className="g14-section" aria-label="Son model çağrıları">
        <p className="g14-note">Son {data.length} model çağrısı. API anahtarı ve bilinen gizli değer kalıpları maskelenir; uzun içerik kısaltılır.</p>
        {data.length === 0 ? (
          <p className="muted ks-empty">Henüz model çağrısı yok.</p>
        ) : (
          <ul className="g14-turns">
            {data.map((r) => (
              <li key={r.id}>
                <button type="button" className={`g14-turn${item?.id === r.id ? ' active' : ''}`} onClick={() => setSelected(r.id)} aria-current={item?.id === r.id}>
                  <span className="g14-turn-top">
                    <span>{formatClock(r.at)}</span>
                    <span className="truncate">{r.model}</span>
                    {r.status !== 'ok' && <span className="g14-tag err">{statusName(r.status)}</span>}
                  </span>
                  <span className="g14-turn-meta">{formatMs(r.durationMs)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
      {item && (
        <section className="g14-section g14-detail" aria-label="İstek ve yanıt">
          <div className="g14-pre-head">
            <h4 className="g14-h4">İstek · {formatDayTime(item.at)}</h4>
            <CopyButton getText={() => item.request} label="İsteği kopyala" />
          </div>
          <pre className="g14-pre" tabIndex={0}>{item.request}</pre>
          <div className="g14-pre-head">
            <h4 className="g14-h4">Yanıt</h4>
            <CopyButton getText={() => item.response} label="Yanıtı kopyala" />
          </div>
          <pre className="g14-pre" tabIndex={0}>{item.response}</pre>
        </section>
      )}
    </div>
  )
}

// ---------- Ayarlar ----------

interface Draft {
  traceExport: boolean
  otlpEndpoint: string
  prometheus: boolean
  prometheusPort: string
  slowToolSec: string
  turnBudgetMin: string
  memoryBudgetMb: string
  alerts: boolean
}

const toDraft = (s: ObservabilitySettings): Draft => ({
  traceExport: s.traceExport,
  otlpEndpoint: s.otlpEndpoint,
  prometheus: s.prometheus,
  prometheusPort: String(s.prometheusPort),
  slowToolSec: String(s.slowToolMs / 1000),
  turnBudgetMin: String(s.turnBudgetMs / 60_000),
  memoryBudgetMb: String(s.memoryBudgetMb),
  alerts: s.alerts,
})

export function SettingsTab() {
  const overview = useSWR('g14:overview', () => api.g14.overview(), { refreshInterval: 2_000, revalidateOnFocus: false })
  const [draft, setDraft] = useState<Draft | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState(false)
  const settings = overview.data?.settings
  useEffect(() => {
    if (settings && draft === null) setDraft(toDraft(settings))
  }, [settings, draft])

  if (overview.error) return <p className="error-text" role="alert">{errorMessage(overview.error)}</p>
  if (!draft || !overview.data) return <p className="muted">Yükleniyor...</p>
  const status = overview.data.exportStatus
  const set = (patch: Partial<Draft>) => {
    setDraft({ ...draft, ...patch })
    setSaved(false)
  }

  const save = async (e: FormEvent) => {
    e.preventDefault()
    setError(null)
    const num = (v: string, f = 1) => Math.round(Number(v.replace(',', '.')) * f)
    const patch: Partial<ObservabilitySettings> = {
      traceExport: draft.traceExport,
      otlpEndpoint: draft.otlpEndpoint.trim(),
      prometheus: draft.prometheus,
      prometheusPort: num(draft.prometheusPort),
      slowToolMs: num(draft.slowToolSec, 1000),
      turnBudgetMs: num(draft.turnBudgetMin, 60_000),
      memoryBudgetMb: num(draft.memoryBudgetMb),
      alerts: draft.alerts,
    }
    try {
      setDraft(toDraft(await api.g14.setSettings(patch)))
      await overview.mutate()
      setSaved(true)
    } catch (err) {
      setError(errorMessage(err))
    }
  }

  return (
    <form className="g14-settings" onSubmit={(e) => void save(e)}>
      <fieldset className="g14-fieldset">
        <legend>İz dışa aktarma (OpenTelemetry)</legend>
        <label className="g14-check">
          <input type="checkbox" checked={draft.traceExport} onChange={(e) => set({ traceExport: e.target.checked })} />
          Tamamlanan tur izlerini OTLP/JSON olarak yerel dosyaya yaz
        </label>
        <p className="g14-note">
          Klasör: <code>{status.traceDir}</code> (traces-GG.jsonl, 14 gün saklanır). Araç özetleri maskelenir; mesaj içeriği yazılmaz.
        </p>
        <label htmlFor="g14-otlp">OTLP/HTTP uç noktası (isteğe bağlı, yalnızca bu makine)</label>
        <input id="g14-otlp" className="g14-input" value={draft.otlpEndpoint} maxLength={300} placeholder="http://127.0.0.1:4318/v1/traces" onChange={(e) => set({ otlpEndpoint: e.target.value })} />
        <p className="g14-note">Boş bırakılırsa gönderim kapalıdır. Yalnızca 127.0.0.1, localhost veya [::1] kabul edilir; yönlendirmeler reddedilir.</p>
        {status.otlpError && <p className="error-text">{status.otlpError}</p>}
        {status.otlpSent > 0 && <p className="muted">{status.otlpSent} iz gönderildi.</p>}
      </fieldset>

      <fieldset className="g14-fieldset">
        <legend>Prometheus metrikleri</legend>
        <label className="g14-check">
          <input type="checkbox" checked={draft.prometheus} onChange={(e) => set({ prometheus: e.target.checked })} />
          Yerel /metrics uç noktasını aç (yalnızca 127.0.0.1)
        </label>
        <label htmlFor="g14-port">Bağlantı noktası</label>
        <input id="g14-port" className="g14-input g14-narrow" inputMode="numeric" value={draft.prometheusPort} onChange={(e) => set({ prometheusPort: e.target.value })} />
        {status.prometheusUrl && (
          <p className="muted">
            Yayında: <code>{status.prometheusUrl}</code> (istek, hata, token, maliyet, gecikme)
          </p>
        )}
        {status.prometheusError && <p className="error-text">{status.prometheusError}</p>}
      </fieldset>

      <fieldset className="g14-fieldset">
        <legend>Eşikler ve uyarılar</legend>
        <label htmlFor="g14-slow">Yavaş araç eşiği (saniye)</label>
        <input id="g14-slow" className="g14-input g14-narrow" inputMode="decimal" value={draft.slowToolSec} onChange={(e) => set({ slowToolSec: e.target.value })} />
        <label htmlFor="g14-turn">Tur süresi bütçesi (dakika)</label>
        <input id="g14-turn" className="g14-input g14-narrow" inputMode="decimal" value={draft.turnBudgetMin} onChange={(e) => set({ turnBudgetMin: e.target.value })} />
        <label htmlFor="g14-mem">Bellek bütçesi (MB, ana süreç)</label>
        <input id="g14-mem" className="g14-input g14-narrow" inputMode="numeric" value={draft.memoryBudgetMb} onChange={(e) => set({ memoryBudgetMb: e.target.value })} />
        <label className="g14-check">
          <input type="checkbox" checked={draft.alerts} onChange={(e) => set({ alerts: e.target.checked })} />
          Bütçe ve yavaş araç uyarılarını bildirim olarak göster
        </label>
      </fieldset>

      <div className="g14-row">
        <button type="submit" className="btn small primary">
          Kaydet
        </button>
        {error && <span className="error-text" role="alert">{error}</span>}
        {saved && <span className="muted" role="status">Kaydedildi (~/.kor/settings.json).</span>}
      </div>
    </form>
  )
}
