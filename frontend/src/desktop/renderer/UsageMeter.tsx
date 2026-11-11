import { Gauge, PanelRight } from 'lucide-react'
import { useId, useState } from 'react'
import useSWR from 'swr'
import type { RateLimitWindow } from '../../core/agent.ts'
import type { UsageBucket, UsagePeriod } from '../../core/usage-history.ts'
import { api, errorMessage } from './api.ts'
import { usePopover } from './use-popover.ts'
import { Tooltip } from './kor-ui/index.ts'
import { openView } from './view-catalog.ts'
import { uiLocale, percentText } from './ui-locale.ts'

const usd = (n: number, digits = 2) => n.toLocaleString(uiLocale(), { style: 'currency', currency: 'USD', minimumFractionDigits: digits, maximumFractionDigits: digits })
const num = (n: number) => n.toLocaleString(uiLocale())
const clock = (ms: number) => new Date(ms).toLocaleTimeString(uiLocale(), { hour: '2-digit', minute: '2-digit', second: '2-digit' })

function usedPercent(w: RateLimitWindow | null): number {
  if (!w) return 0
  return Math.min(100, Math.max(0, Math.round(((w.limit - w.remaining) / w.limit) * 100)))
}

function LimitRow({ label, window, fallbackLimit }: { label: string; window: RateLimitWindow | null; fallbackLimit: number | null }) {
  const pct = usedPercent(window)
  const limit = window?.limit ?? fallbackLimit
  return (
    <div className="usage-row">
      <div className="usage-line">
        <span>{label}</span>
        <span className="usage-meta">
          {window ? `Sıfırlanma: ${clock(window.resetAt)}` : limit ? `Sınır: ${num(limit)}/dk` : 'Henüz istek yok'}
          <span className="usage-pct">{percentText(pct)}</span>
        </span>
      </div>
      <div className="bar" role="progressbar" aria-label={label} aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
        <span className="bar-fill" style={{ width: `${Math.max(pct, window ? 2 : 0)}%` }} />
      </div>
    </div>
  )
}

const CHART_W = 364
const CHART_H = 88
const BAR_GAP = 2

/** Yalnızca üst köşeleri yuvarlak, tabana oturan çubuk. */
function barPath(x: number, y: number, w: number, h: number): string {
  const r = Math.min(4, w / 2, h)
  return `M${x},${y + h}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + h}Z`
}

const shortDate = (iso: string) => new Date(`${iso}T00:00:00`).toLocaleDateString(uiLocale(), { day: 'numeric', month: 'short' })

function UsageChart({ buckets, period }: { buckets: UsageBucket[]; period: UsagePeriod }) {
  const max = Math.max(0, ...buckets.map((b) => b.usd + b.diem))
  const barW = buckets.length > 0 ? (CHART_W - BAR_GAP * (buckets.length - 1)) / buckets.length : 0
  const first = buckets[0]
  const last = buckets.at(-1)
  const label = (b: UsageBucket) => (period === 'week' ? `${shortDate(b.start)} haftası` : shortDate(b.start))
  return (
    <figure className="g7-chart">
      <svg
        width="100%"
        viewBox={`0 0 ${CHART_W} ${CHART_H + 1}`}
        role="img"
        aria-label={`${period === 'day' ? 'Günlük' : 'Haftalık'} harcama grafiği, en yüksek ${usd(max)}`}
      >
        {buckets.map((b, i) => {
          const value = b.usd + b.diem
          const h = max > 0 ? Math.max(value > 0 ? 2 : 0, (value / max) * (CHART_H - 4)) : 0
          const x = i * (barW + BAR_GAP)
          return (
            <g key={b.start} className="g7-bar">
              {/* Geniş, görünmez isabet alanı: ince çubuklarda da üzerine gelince değer görünür. */}
              <rect x={x} y={0} width={barW + BAR_GAP} height={CHART_H} className="g7-bar-hit" />
              {h > 0 && <path d={barPath(x, CHART_H - h, barW, h)} className="g7-bar-fill" />}
              <title>{`${label(b)}: ${usd(b.usd)} + ${num(b.diem)} DIEM`}</title>
            </g>
          )
        })}
        <line x1={0} x2={CHART_W} y1={CHART_H + 0.5} y2={CHART_H + 0.5} className="g7-axis" />
      </svg>
      <figcaption className="g7-chart-axis">
        <span>{first ? shortDate(first.start) : ''}</span>
        <span>En yüksek: {usd(max)}</span>
        <span>{last ? shortDate(last.start) : ''}</span>
      </figcaption>
      <table className="sr-only">
        <caption>Harcama ({period === 'day' ? 'günlük' : 'haftalık'})</caption>
        <thead>
          <tr>
            <th scope="col">Dönem</th>
            <th scope="col">USD</th>
            <th scope="col">DIEM</th>
          </tr>
        </thead>
        <tbody>
          {buckets.map((b) => (
            <tr key={b.start}>
              <th scope="row">{label(b)}</th>
              <td>{usd(b.usd)}</td>
              <td>{num(b.diem)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
  )
}

/** Venice /billing/usage-analytics: günlük (30 gün) veya haftalık (12 hafta) harcama. */
function UsageHistorySection() {
  const id = useId()
  const [period, setPeriod] = useState<UsagePeriod>('day')
  const { data, error, isLoading } = useSWR(['g7:usage', period], () => api.g7GetUsageHistory(period), { revalidateOnFocus: false, keepPreviousData: true, dedupingInterval: 5 * 60_000 })
  return (
    <div className="usage-section" aria-labelledby={`${id}-title`}>
      <div className="g7-usage-head">
        <span id={`${id}-title`} className="usage-title">Harcama geçmişi</span>
        <div className="g7-tabs g7-tabs-small" role="tablist" aria-label="Dönem">
          {(['day', 'week'] as const).map((p) => (
            <button key={p} type="button" role="tab" aria-selected={period === p} className="g7-tab" onClick={() => setPeriod(p)}>
              {p === 'day' ? 'Günlük' : 'Haftalık'}
            </button>
          ))}
        </div>
      </div>
      {isLoading && !data && <span className="muted">Yükleniyor...</span>}
      {error && <span className="error-text">{errorMessage(error)}</span>}
      {data && (
        <>
          {data.buckets.some((b) => b.usd + b.diem > 0) ? (
            <UsageChart buckets={data.buckets} period={data.period} />
          ) : (
            <span className="muted">Bu dönemde harcama yok.</span>
          )}
          <span className="muted">
            {data.period === 'day' ? 'Son 30 gün' : 'Son 12 hafta'}: {usd(data.totalUsd)} · {num(Math.round(data.totalDiem * 100) / 100)} DIEM
          </span>
          {data.topModels.length > 0 && (
            <ol className="g7-top-models">
              {data.topModels.map((m) => (
                <li key={m.name}>
                  <span className="truncate">{m.name}</span>
                  <span className="muted">{m.diem > 0 ? `${usd(m.usd)} + ${num(m.diem)} DIEM` : usd(m.usd)}</span>
                </li>
              ))}
            </ol>
          )}
        </>
      )}
    </div>
  )
}

/** Halka gösterge + Claude'daki "Plan kullanım sınırları" benzeri açılır pencere. */
export function UsageMeter({ refreshKey, sessionId }: { refreshKey: number; sessionId: string | null }) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const { data, error, isLoading } = useSWR(['usage', refreshKey, open, sessionId], () => api.getUsage(sessionId), { keepPreviousData: true })
  const pct = Math.max(usedPercent(data?.live?.requests ?? null), usedPercent(data?.live?.tokens ?? null))
  const hasLive = Boolean(data?.live?.requests ?? data?.live?.tokens)
  const radius = 7
  const circumference = 2 * Math.PI * radius

  return (
    <div className="popover-anchor" ref={ref}>
      <Tooltip text={hasLive ? `Kullanım ve sınırlar: dakikalık sınırın %${pct} kullanıldı` : 'Kullanım ve sınırlar (bakiye, harcama geçmişi)'}>
        <button
          type="button"
          className="usage-trigger"
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-label={hasLive ? `Kullanım ve sınırlar: dakikalık sınırın %${pct} kullanıldı` : 'Kullanım ve sınırlar'}
        >
          {/* Sınır verisi gelene kadar boş halka yerine anlamlı ikon */}
          {!hasLive ? <Gauge size={16} aria-hidden="true" /> : <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
            <circle cx="9" cy="9" r={radius} className="ring-track" />
            <circle
              cx="9"
              cy="9"
              r={radius}
              className="ring-fill"
              strokeDasharray={circumference}
              strokeDashoffset={circumference * (1 - Math.max(pct, 4) / 100)}
            />
          </svg>}
        </button>
      </Tooltip>

      {open && (
        <div className="menu usage-pop pop-in" role="dialog" aria-label="Kullanım sınırları">
          <div className="usage-head">
            <span className="muted">Kullanım sınırları{data?.tier ? ` · ${data.tier === 'paid' ? 'Ücretli' : data.tier}` : ''}</span>
            <button
              type="button"
              className="link-btn kor-meter-details"
              onClick={() => {
                setOpen(false)
                openView('cost')
              }}
            >
              <PanelRight size={14} aria-hidden="true" />
              Ayrıntılar
            </button>
          </div>

          {isLoading && !data && <p className="muted">Yükleniyor...</p>}
          {error && <p className="error-text">{errorMessage(error)}</p>}

          {data && (
            <>
              <LimitRow label="İstek sınırı (dakika)" window={data.live?.requests ?? null} fallbackLimit={data.modelLimits?.rpm ?? null} />
              <LimitRow label="Token sınırı (dakika)" window={data.live?.tokens ?? null} fallbackLimit={data.modelLimits?.tpm ?? null} />

              <div className="usage-section">
                <span className="usage-title">Bakiye</span>
                {data.balances ? (
                  <span className="muted">
                    {usd(data.balances.usd)} · {num(data.balances.diem)} DIEM
                    {data.balances.bundled > 0 ? ` · ${usd(data.balances.bundled)} paket kredi` : ''}
                  </span>
                ) : (
                  <span className="muted">{data.accountError ?? 'Bilinmiyor'}</span>
                )}
              </div>

              <div className="usage-section">
                <span className="usage-title">Bu oturum</span>
                <span className="muted">
                  {num(data.session.promptTokens)} girdi + {num(data.session.completionTokens)} çıktı token
                  {data.session.costUsd !== null && data.session.promptTokens + data.session.completionTokens > 0
                    ? ` · ~${usd(data.session.costUsd, 4)}`
                    : ''}
                </span>
              </div>

              <UsageHistorySection />

              {data.nextEpochBegins && (
                <p className="usage-foot">
                  Günlük dönem yenilenmesi:{' '}
                  {new Date(data.nextEpochBegins).toLocaleString(uiLocale(), { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit', timeZoneName: 'short' })}
                </p>
              )}
            </>
          )}
        </div>
      )}
    </div>
  )
}
