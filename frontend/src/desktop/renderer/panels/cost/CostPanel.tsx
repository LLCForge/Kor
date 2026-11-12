// Maliyet paneli (§5.2 canlı pano, §5.1 bütçe/yönlendirme, §15.10 kota, `kor bench`). Rakamlar sağlayıcı
// verisinden ya da katalog fiyatından gelir; tahmin "≈" ile gösterilir. Tur bitince pano yenilenir.
import { Check, Download, X } from 'lucide-react'
import { useCallback, useId, useState } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { CostReport, CostRow } from '../../../ipc-cost.ts'
import { errorMessage } from '../../api.ts'
import { Columns, HBars } from '../../g14-charts.tsx'
import { formatTokens } from '../../g7-format.ts'
import type { KorPanelProps } from '../registry.ts'
import { costApi, keyPrefix, packagesApi, useRefreshOnTurnDone } from './api.ts'
import { BenchSection } from './BenchSection.tsx'
import { ModelRef } from '../../ModelRef.tsx'
import { ProviderLogo } from '../providers/ProviderLogo.tsx'
import { BudgetSettings } from './BudgetSettings.tsx'
import { ConsensusSection } from './ConsensusSection.tsx'
import { budgetState, byProvider, money, routeLabel, rowMoney, type BudgetLevel } from './shape.ts'
import { uiLocale, percentText } from '../../ui-locale.ts'

const TABS = [
  { id: 'dashboard', label: 'Pano' },
  { id: 'budget', label: 'Bütçe ve yönlendirme' },
  { id: 'why', label: 'Kararlar' },
  { id: 'quota', label: 'Kota' },
  { id: 'consensus', label: 'Consensus' },
  { id: 'bench', label: 'Bench' },
] as const

type Tab = (typeof TABS)[number]['id']

const LEVEL_TEXT: Record<BudgetLevel, string> = { none: 'limit yok', ok: 'limit içinde', alert: 'uyarı eşiğinde', over: 'limit aşıldı' }

const isCostKey = keyPrefix('cost:')

export function CostPanel(_props: KorPanelProps) {
  const id = useId()
  const [tab, setTab] = useState<Tab>('dashboard')
  const { mutate } = useSWRConfig()
  const refresh = useCallback(() => void mutate(isCostKey), [mutate])
  useRefreshOnTurnDone(refresh)

  return (
    <div className="g2c-panel" data-kor-event="kor:event">
      <div className="g14-tabs" role="tablist" aria-label="Maliyet bölümleri">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            id={`${id}-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls={`${id}-body`}
            className={`g14-tab${tab === t.id ? ' active' : ''}`}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="g2c-body" role="tabpanel" id={`${id}-body`} aria-labelledby={`${id}-${tab}`}>
        {tab === 'dashboard' && <Dashboard />}
        {tab === 'budget' && <BudgetSettings />}
        {tab === 'why' && <Decisions />}
        {tab === 'quota' && <Quota />}
        {tab === 'consensus' && <ConsensusSection />}
        {tab === 'bench' && <BenchSection />}
      </div>
    </div>
  )
}

function Loading({ error }: { error: unknown }) {
  return error ? <p className="error-text" role="alert">{errorMessage(error)}</p> : <p className="muted">Yükleniyor...</p>
}

const tokensText = (r: CostRow) => `${formatTokens(r.prompt)}↑ ${formatTokens(r.completion)}↓`

function bars(rows: readonly CostRow[]) {
  return rows.map((r) => ({ key: r.model, value: r.usd, text: rowMoney(r), title: `${r.model}: ${rowMoney(r)} · ${r.requests} istek · ${tokensText(r)}` }))
}

function LimitCard({ title, spent, limit, alert }: { title: string; spent: CostRow; limit: number; alert: number }) {
  const s = budgetState(spent.usd, limit, alert)
  return (
    <div className="g14-card">
      <span className="g14-card-title">{title}</span>
      <span className="g14-card-value">
        {rowMoney(spent)}
        {limit > 0 && <span className="muted"> / {money(limit, false)}</span>}
        {(s.level === 'alert' || s.level === 'over') && <span className={`g14-tag ${s.level === 'over' ? 'err' : 'warn'}`}>{LEVEL_TEXT[s.level]}</span>}
      </span>
      {s.pct !== null && (
        <div className="bar" role="progressbar" aria-label={`${title} limit kullanımı`} aria-valuenow={Math.min(s.pct, 100)} aria-valuemin={0} aria-valuemax={100}>
          <span className={`bar-fill g2c-${s.level}`} style={{ width: `${Math.min(100, Math.max(s.pct, 2))}%` }} />
        </div>
      )}
      <span className="g14-card-detail">
        {spent.requests} istek · {s.pct !== null ? `%${s.pct} · ` : ''}
        {LEVEL_TEXT[s.level]}
      </span>
    </div>
  )
}

function Dashboard() {
  const [days, setDays] = useState(30)
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null)
  const summary = useSWR(['cost:summary', days], () => costApi.summary(days), { keepPreviousData: true, revalidateOnFocus: false })
  const settings = useSWR(['cost:settings'], () => costApi.settings(), { revalidateOnFocus: false })
  const packages = useSWR(['cost:packages', days], () => packagesApi.cost(days), { revalidateOnFocus: false })

  const exportFile = async (format: 'csv' | 'json') => {
    setNote(null)
    try {
      const { path } = await costApi.exportFile(format, days)
      if (path) setNote({ ok: true, text: `Kaydedildi: ${path}` })
    } catch (err) {
      setNote({ ok: false, text: errorMessage(err) })
    }
  }

  if (!summary.data) return <Loading error={summary.error} />
  const r: CostReport = summary.data.report
  const alert = settings.data?.budget.alert_at_percent ?? 80
  const providers = byProvider(r.rows)
  const pkgRows = packages.data?.rows.filter((p) => p.model !== '-') ?? []

  return (
    <div className="g2c-stack" data-kor-cmd="cost.summary">
      <div className="g7-inline">
        <label htmlFor="g2c-days">Dönem</label>
        <select id="g2c-days" className="g7-select" value={days} onChange={(e) => setDays(Number(e.target.value))}>
          <option value={1}>Son 1 gün</option>
          <option value={7}>Son 7 gün</option>
          <option value={30}>Son 30 gün</option>
          <option value={90}>Son 90 gün</option>
          <option value={365}>Son 365 gün</option>
        </select>
        <span className="g7-grow" />
        <button type="button" className="btn small" onClick={() => void exportFile('csv')} data-kor-cmd="cost.exportFile">
          <Download size={14} aria-hidden="true" /> CSV
        </button>
        <button type="button" className="btn small" onClick={() => void exportFile('json')} data-kor-cmd="cost.exportFile">
          <Download size={14} aria-hidden="true" /> JSON
        </button>
      </div>
      {note && (
        <p className={note.ok ? 'g7-note' : 'error-text'} role="status" translate="no">
          {note.text}
        </p>
      )}

      <div className="g14-cards">
        <LimitCard title="Bugün" spent={r.today} limit={r.daily_limit_usd} alert={alert} />
        <LimitCard title="Bu ay" spent={r.month} limit={r.monthly_limit_usd} alert={alert} />
        <div className="g14-card">
          <span className="g14-card-title">Ay sonu projeksiyonu</span>
          <span className="g14-card-value">{money(r.projection_usd, true)}</span>
          <span className="g14-card-detail">Bu ayın günlük ortalamasından tahmin</span>
        </div>
        <div className="g14-card">
          <span className="g14-card-title">Toplam (son {r.days} gün)</span>
          <span className="g14-card-value">{rowMoney(r.total)}</span>
          <span className="g14-card-detail">
            {r.total.requests} istek · {tokensText(r.total)}
          </span>
        </div>
        {r.media && r.media.requests > 0 && (
          <div className="g14-card">
            <span className="g14-card-title">Medya (görsel, video, ses)</span>
            <span className="g14-card-value">{rowMoney(r.media)}</span>
            <span className="g14-card-detail">{r.media.requests} üretim · toplamın parçası</span>
          </div>
        )}
      </div>

      {(r.total.estimated || r.month.estimated) && <p className="g7-note">≈ işaretli tutarlar sağlayıcı yerine katalog fiyatından hesaplandı.</p>}
      {r.total.unpriced > 0 && <p className="g7-note">{r.total.unpriced} isteğin fiyatı bilinmiyor; toplamlara girmedi.</p>}

      {r.rows.length === 0 ? (
        <p className="muted g14-empty">Bu dönemde model çağrısı kaydı yok. Bir sohbet turu bitince burada görünür.</p>
      ) : (
        <>
          <section className="g14-section" aria-label="Model bazında">
            <h3 className="g14-h">Model bazında</h3>
            <HBars items={bars(r.rows)} label="Model bazında maliyet" />
            <div className="g7-table-wrap">
              <table className="g7-table">
                <thead>
                  <tr>
                    <th scope="col">Model</th>
                    <th scope="col" className="num">Tutar</th>
                    <th scope="col" className="num">İstek</th>
                    <th scope="col" className="num">Token</th>
                  </tr>
                </thead>
                <tbody>
                  {r.rows.map((row) => (
                    <tr key={row.model}>
                      <td><ModelRef model={row.model} /></td>
                      <td className="num">{rowMoney(row)}</td>
                      <td className="num">{row.requests}</td>
                      <td className="num">{tokensText(row)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
          <section className="g14-section" aria-label="Sağlayıcı bazında">
            <h3 className="g14-h">Sağlayıcı bazında</h3>
            <HBars items={bars(providers)} label="Sağlayıcı bazında maliyet" />
          </section>
        </>
      )}

      <section className="g14-section" aria-label="Paket bazında" data-kor-cmd="packages.cost">
        <h3 className="g14-h">Paket bazında</h3>
        {packages.error ? (
          <p className="muted g14-empty">{errorMessage(packages.error)}</p>
        ) : !packages.data ? (
          <p className="muted">Yükleniyor...</p>
        ) : pkgRows.length === 0 ? (
          <p className="muted g14-empty">Bu çalışma alanında paketlere düşen kayıt yok (monorepo değil ya da bu dönemde istek yok).</p>
        ) : (
          <HBars items={bars(pkgRows)} label="Paket bazında maliyet" />
        )}
      </section>
    </div>
  )
}

function Decisions() {
  const why = useSWR(['cost:why'], () => costApi.why(50), { revalidateOnFocus: false })
  if (!why.data) return <Loading error={why.error} />
  const list = [...why.data.decisions].reverse()
  if (list.length === 0)
    return <p className="muted g14-empty">Henüz yönlendirme kararı yok. Akıllı yönlendirme ([routing] enabled) açıkken her görevde model seçimi buraya yazılır.</p>
  return (
    <ul className="g2c-decisions" aria-label="Yönlendirme kararları" data-kor-cmd="cost.why">
      {list.map((d) => (
        <li key={d.id} className="g2c-decision">
          <div className="g7-inline">
            <span className="g14-num">{new Date(d.at).toLocaleString(uiLocale(), { dateStyle: 'short', timeStyle: 'short' })}</span>
            <span className="g14-tag">{routeLabel(d.category)}</span>
            {d.model && <ModelRef model={d.model} />}
            <span className="g7-grow" />
            {d.outcome === true && (
              <span className="g2c-ok">
                <Check size={14} aria-hidden="true" /> başarılı
              </span>
            )}
            {d.outcome === false && (
              <span className="g2c-fail">
                <X size={14} aria-hidden="true" /> başarısız
              </span>
            )}
          </div>
          {d.reason && <p className="g14-hint" translate="no">{d.reason}</p>}
          {d.task && <p className="g14-sub truncate" translate="no">{d.task}</p>}
        </li>
      ))}
    </ul>
  )
}

const count = (rem: number | null, lim: number | null) => (rem === null ? '—' : lim === null ? formatTokens(rem) : `${formatTokens(rem)} / ${formatTokens(lim)}`)

function Quota() {
  const q = useSWR(['cost:quota'], () => costApi.quota(), { revalidateOnFocus: false })
  if (!q.data) return <Loading error={q.error} />
  const r = q.data.report
  const usedSeries = r.keys.map((k) => ({ label: `${k.provider} ${k.key}`, value: k.used_percent, title: `${k.provider} ${k.key}: %${k.used_percent ?? '?'}` }))
  return (
    <div className="g2c-stack" data-kor-cmd="cost.quota">
      <section className="g14-section" aria-label="Anahtar kotaları">
        <h3 className="g14-h">Anahtar başına kota</h3>
        {r.keys.length === 0 ? (
          <p className="muted g14-empty">Henüz kota başlığı görülmedi; sağlayıcı yanıtlarında rate-limit başlıkları geldikçe dolar.</p>
        ) : (
          <>
            <div className="g7-table-wrap">
              <table className="g7-table">
                <thead>
                  <tr>
                    <th scope="col">Anahtar</th>
                    <th scope="col" className="num">Kalan istek</th>
                    <th scope="col" className="num">Kalan token</th>
                    <th scope="col">Sıfırlanma</th>
                    <th scope="col" className="num">Kullanım</th>
                  </tr>
                </thead>
                <tbody>
                  {r.keys.map((k) => (
                    <tr key={`${k.provider}-${k.key}`}>
                      <td translate="no">
                        <ProviderLogo provider={k.provider} size={14} /> <span className="g7-mono">{k.provider}</span> <span className="muted">{k.key}</span>
                        {k.low && <span className="g14-tag warn"> az kaldı</span>}
                      </td>
                      <td className="num">{count(k.remaining_requests, k.limit_requests)}</td>
                      <td className="num">{count(k.remaining_tokens, k.limit_tokens)}</td>
                      <td translate="no">{k.reset ?? '—'}</td>
                      <td className="num">{k.used_percent === null ? '—' : percentText(k.used_percent)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <Columns points={usedSeries} max={100} label="Anahtar başına kullanım yüzdesi" format={(v) => percentText(v)} />
          </>
        )}
      </section>

      {r.openrouter && (
        <p className="g7-note">
          OpenRouter kredisi: {money(r.openrouter[1], false)} kullanıldı / {money(r.openrouter[0], false)}
        </p>
      )}

      {q.data.proxy && (
        <section className="g14-section" aria-label="Proxy hesapları">
          <h3 className="g14-h">Proxy hesapları</h3>
          <p className="muted" translate="no">{q.data.proxy.url}</p>
          {q.data.proxy.accounts.length === 0 ? (
            <p className="muted g14-empty">Ağ geçidi çalışıyor; henüz istek yok.</p>
          ) : (
            <div className="g7-table-wrap">
              <table className="g7-table">
                <thead>
                  <tr>
                    <th scope="col">Hesap</th>
                    <th scope="col" className="num">İstek</th>
                    <th scope="col" className="num">Hata</th>
                    <th scope="col" className="num">Hız sınırı</th>
                    <th scope="col" className="num">Token</th>
                  </tr>
                </thead>
                <tbody>
                  {q.data.proxy.accounts.map((a) => (
                    <tr key={a.account}>
                      <td translate="no" className="g7-mono">{a.account}</td>
                      <td className="num">{a.requests}</td>
                      <td className="num">{a.failed}</td>
                      <td className="num">{a.rateLimited}</td>
                      <td className="num">{formatTokens(a.tokens)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      )}

      <section className="g14-section" aria-label="Son 24 saat">
        <h3 className="g14-h">Son 24 saat</h3>
        {r.last_24h.length === 0 ? (
          <p className="muted g14-empty">Son 24 saatte istek yok.</p>
        ) : (
          <HBars
            items={r.last_24h.map((d) => ({ key: d.provider, value: d.requests, text: `${d.requests} istek · ${formatTokens(d.prompt)}↑ ${formatTokens(d.completion)}↓` }))}
            label="Son 24 saat sağlayıcı bazında istek"
          />
        )}
      </section>

      {r.failover.length > 0 && (
        <section className="g14-section" aria-label="Yedekleme sırası">
          <h3 className="g14-h">Yedekleme sırası</h3>
          <ol className="g2c-failover" translate="no">
            {r.failover.map((f) => (
              <li key={f} className="g7-mono">{f}</li>
            ))}
          </ol>
        </section>
      )}
      {r.notes.map((n) => (
        <p key={n} className="g7-note" translate="no">{n}</p>
      ))}
    </div>
  )
}
