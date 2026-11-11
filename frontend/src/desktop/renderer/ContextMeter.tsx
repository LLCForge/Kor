import { ChevronsDownUp, PanelRight } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import type { ContextInfo, ContextView } from '../ipc-g6.ts'
import { api, errorMessage } from './api.ts'
import { formatTokens, percent } from './g6-format.ts'
import { usePopover } from './use-popover.ts'
import { Tooltip } from './kor-ui/index.ts'
import { openView } from './view-catalog.ts'
import { percentText } from './ui-locale.ts'

interface ContextMeterProps {
  /** null: henüz gönderilmemiş yeni sohbet */
  sessionId: string | null
  running: boolean
}

const BUDGET_PARTS = [
  { key: 'systemTokens', label: 'Sistem istemi', className: 'g31-seg-system' },
  { key: 'memoryTokens', label: 'Bellek', className: 'g31-seg-memory' },
  { key: 'toolsTokens', label: 'Araç tanımları', className: 'g31-seg-tools' },
  { key: 'historyTokens', label: 'Konuşma', className: 'g31-seg-history' },
] as const

/** Bağlam bütçesi: pencerenin hangi payının neye gittiği (tahmini) ve kalan boş alan. */
function BudgetBreakdown({ info, limit }: { info: ContextView; limit: number | null }) {
  const b = info.breakdown
  if (!b) return null
  const total = BUDGET_PARTS.reduce((sum, p) => sum + b[p.key], 0)
  const scale = Math.max(limit ?? total, total, 1)
  return (
    <div className="usage-section">
      <span className="usage-title">Bağlam bütçesi (tahmini)</span>
      <div className="g31-budget-bar" role="img" aria-label="Bağlam bütçesi dağılımı">
        {BUDGET_PARTS.map((p) => (
          <span key={p.key} className={p.className} style={{ width: `${(b[p.key] / scale) * 100}%` }} />
        ))}
      </div>
      <ul className="g31-budget-list">
        {BUDGET_PARTS.map((p) => (
          <li key={p.key}>
            <span className={`g31-dot ${p.className}`} aria-hidden="true" />
            <span>{p.label}</span>
            <span className="muted">~{formatTokens(b[p.key])}</span>
          </li>
        ))}
        {limit !== null && (
          <li>
            <span className="g31-dot" aria-hidden="true" />
            <span>Boş</span>
            <span className="muted">~{formatTokens(Math.max(0, limit - total))}</span>
          </li>
        )}
      </ul>
    </div>
  )
}

function EfficiencyReport({ info }: { info: ContextView }) {
  const net = info.savedTokens - info.compactionCostTokens
  return (
    <div className="usage-section">
      <span className="usage-title">Token verimliliği (bu oturum)</span>
      <span className="muted">
        {info.compactions > 0
          ? `Sıkıştırma: ~${formatTokens(info.savedTokens)} token gönderilmedi (${info.compactions} sıkıştırma, özet maliyeti ${formatTokens(info.compactionCostTokens)}; net ~${formatTokens(Math.max(0, net))})`
          : 'Sıkıştırma: henüz yapılmadı'}
      </span>
      <span className="muted">
        {info.mapSavedTokens !== null
          ? `Proje haritası: kod tabanını baştan okumak yerine ~${formatTokens(info.mapSavedTokens)} token'lık keşif yeniden kullanıldı`
          : 'Proje haritası: bu sohbette yok (kenar çubuğundan oluşturulabilir)'}
      </span>
    </div>
  )
}

/** Mesaj kutusundaki bağlam doluluğu halkası; açılır pencerede önbellek isabeti, verimlilik ve "Sıkıştır". */
export function ContextMeter({ sessionId, running }: ContextMeterProps) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const { data, error, mutate } = useSWR(['g6:context', sessionId, open], () => api.getContextInfo(sessionId), {
    keepPreviousData: true,
    revalidateOnFocus: false,
  })
  // Ana süreç her model çağrısından sonra olay gönderir; başka sohbetin değeri bu sohbette gösterilmez.
  const [live, setLive] = useState<{ sessionId: string; info: ContextInfo } | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)

  useEffect(
    () =>
      api.onEvent((event) => {
        if (event.sessionId !== sessionId) return
        if (event.type === 'g6:context') setLive({ sessionId: event.sessionId, info: event.info })
        if (event.type === 'g6:compacted') void mutate()
      }),
    [sessionId, mutate],
  )

  const info: ContextView | undefined = data && live?.sessionId === sessionId ? { ...data, ...live.info } : data
  const limit = info?.limitTokens ?? null
  const pct = info && limit ? percent(info.usedTokens, limit) : 0
  // Eşik artık gerçek token değeri: yüzde metni hesaplanır, tavandan bağımsız gösterilmez.
  const trigger = info?.compactAtTokens ?? null
  const threshold = trigger !== null && limit ? percent(trigger, limit) : Math.round((info?.autoCompactRatio ?? 0.7) * 100)
  const level = pct >= threshold ? 'full' : pct >= threshold - 15 ? 'high' : 'normal'
  const cachePct = info ? percent(info.cachedTokens, info.promptTokens) : 0
  const radius = 7
  const circumference = 2 * Math.PI * radius

  const compact = async () => {
    if (!sessionId) return
    setBusy(true)
    setMessage(null)
    try {
      const result = await api.compactSession(sessionId)
      setMessage(
        result.compacted
          ? `Sıkıştırıldı: ${formatTokens(result.beforeTokens)} → ${formatTokens(result.afterTokens)} token`
          : (result.reason ?? 'Sıkıştırılmadı'),
      )
      await mutate()
    } catch (err) {
      setMessage(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  // Sohbette henüz token yokken boş halka ve "%0" anlamsız; ilk model çağrısından sonra görünür.
  if (!info || info.usedTokens <= 0) return null

  return (
    <div className="popover-anchor" ref={ref}>
      <Tooltip text={limit ? `Bağlam penceresi: %${pct} dolu (sıkıştırma eşiği %${threshold})` : 'Bağlam penceresi'}>
        <button
          type="button"
          className={`usage-trigger g6-context-trigger g6-level-${level}`}
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-label={limit ? `Bağlam penceresi: %${pct} dolu` : 'Bağlam penceresi'}
        >
          <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
            <circle cx="9" cy="9" r={radius} className="ring-track" />
            <circle
              cx="9"
              cy="9"
              r={radius}
              className="ring-fill g6-ring-fill"
              strokeDasharray={circumference}
              strokeDashoffset={circumference * (1 - Math.max(pct, 3) / 100)}
            />
          </svg>
          {limit !== null && <span className="g6-context-pct">{percentText(pct)}</span>}
        </button>
      </Tooltip>

      {open && (
        <div className="menu usage-pop pop-in" role="dialog" aria-label="Bağlam penceresi">
          <div className="usage-head">
            <span className="muted">Bağlam penceresi</span>
            <button
              type="button"
              className="link-btn kor-meter-details"
              onClick={() => {
                setOpen(false)
                openView('context')
              }}
            >
              <PanelRight size={14} aria-hidden="true" />
              Ayrıntılar
            </button>
            <button
              type="button"
              className="btn small"
              onClick={() => void compact()}
              disabled={!sessionId || running || busy}
              title={running ? 'Sohbet çalışırken sıkıştırılamaz' : 'Eski turları özetle (/compact)'}
            >
              <ChevronsDownUp size={14} aria-hidden="true" />
              {busy ? 'Sıkıştırılıyor...' : 'Sıkıştır'}
            </button>
          </div>

          {error && <p className="error-text">{errorMessage(error)}</p>}
          {!info && !error && <p className="muted">Yükleniyor...</p>}

          {info && (
            <>
              <div className="usage-row">
                <div className="usage-line">
                  <span>
                    {formatTokens(info.usedTokens)} / {limit ? formatTokens(limit) : '?'} token
                  </span>
                  <span className="usage-meta">
                    <span className="usage-pct">{percentText(pct)}</span>
                  </span>
                </div>
                <div className="bar" role="progressbar" aria-label="Bağlam doluluğu" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
                  <span className={`bar-fill g6-bar-${level}`} style={{ width: `${Math.max(pct, info.usedTokens > 0 ? 2 : 0)}%` }} />
                </div>
                <span className="g6-note">
                  {info.usedTokens >= (info.compactAtTokens ?? Number.POSITIVE_INFINITY)
                    ? 'Eşik aşıldı: sıradaki model çağrısında otomatik sıkıştırılır.'
                    : trigger !== null
                      ? `${formatTokens(trigger)} token'da otomatik sıkıştırma yapılır (pencerenin %${threshold}'i).`
                      : 'Modelin bağlam sınırı bilinmiyor; otomatik sıkıştırma kapalı.'}
                </span>
              </div>

              <BudgetBreakdown info={info} limit={limit} />

              <div className="usage-section">
                <span className="usage-title">Önbellek isabeti</span>
                <span className="muted">
                  {info.promptTokens > 0
                    ? `%${cachePct} · ${formatTokens(info.cachedTokens)} / ${formatTokens(info.promptTokens)} girdi token önbellekten (indirimli)`
                    : 'Henüz model çağrısı yok'}
                </span>
              </div>

              <EfficiencyReport info={info} />
            </>
          )}

          {message && (
            <p className="usage-foot" role="status">
              {message}
            </p>
          )}
        </div>
      )}
    </div>
  )
}
