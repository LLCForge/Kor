// Mesaj kutusundaki maliyet göstergesi: gönderim öncesi girdi token + maliyet tahmini (48-ek), akış sırasında canlı görev
// maliyeti ve kesme (54), önbellekten gelen token ve kazanılan tutar (49-ek), görev tavanı (53).
import { Coins } from 'lucide-react'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import type { TurnCostInfo } from '../../ipc-g9.ts'
import { api, errorMessage } from '../api.ts'
import { formatTokens, formatUsd } from '../g7-format.ts'
import { openG7Dialog } from '../g7-bus.ts'
import { usePopover } from '../use-popover.ts'
import { formatCost } from './use-g9.ts'
import { Tooltip } from '../kor-ui/index.ts'

interface CostChipProps {
  /** null: henüz gönderilmemiş yeni sohbet */
  sessionId: string | null
  model: string
  /** Taslak + ekler, yaklaşık (g2 sayacı) */
  draftTokens: number
  running: boolean
  onAbort: () => void
}

export function CostChip({ sessionId, model, draftTokens, running, onAbort }: CostChipProps) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const { data, error, mutate } = useSWR(['g9:cost', sessionId, model, open], () => api.g9.costInfo(sessionId, model), {
    keepPreviousData: true,
    revalidateOnFocus: false,
  })
  const [live, setLive] = useState<{ sessionId: string; info: TurnCostInfo } | null>(null)

  useEffect(
    () =>
      api.onEvent((event) => {
        if (event.sessionId !== sessionId) return
        if (event.type === 'g9:cost') {
          const { type: _t, sessionId: sid, ...info } = event
          setLive({ sessionId: sid, info })
        }
        if (event.type === 'done' || event.type === 'error') void mutate()
      }),
    [sessionId, mutate],
  )

  const price = data?.price ?? null
  const inputTokens = (data?.contextTokens ?? 0) + draftTokens
  const estimate = price ? (inputTokens * price.input) / 1e6 : null
  const turn = live && live.sessionId === sessionId ? live.info : null
  const cachedPct = data && data.promptTokens > 0 ? Math.round((data.cachedTokens / data.promptTokens) * 100) : 0
  const savings = price && price.cacheInput !== null && data ? (data.cachedTokens * Math.max(0, price.input - price.cacheInput)) / 1e6 : null
  const capHit = turn?.capUsd != null && turn.turnUsd >= turn.capUsd

  const label = running && turn ? `${formatCost(turn.turnUsd)}${turn.live ? '…' : ''}` : estimate !== null ? `≈ ${formatCost(estimate)}` : '$ ?'

  return (
    <div className="popover-anchor" ref={ref}>
      <Tooltip text="Maliyet">
        <button
          type="button"
          className={`usage-trigger g9-cost-trigger${capHit ? ' warn' : ''}`}
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-label={running ? `Bu görevin maliyeti ${label}` : `Gönderim öncesi tahmini girdi maliyeti ${label}`}
        >
          <Coins size={14} aria-hidden="true" />
          <span className="g9-cost-label">{label}</span>
        </button>
      </Tooltip>

      {open && (
        <div className="menu usage-pop pop-in" role="dialog" aria-label="Maliyet">
          {error && <p className="error-text">{errorMessage(error)}</p>}
          <div className="usage-section">
            <span className="usage-title">Gönderim öncesi tahmin</span>
            <span className="muted">
              Girdi ≈ {formatTokens(inputTokens)} token (bağlam {formatTokens(data?.contextTokens ?? 0)} + taslak {formatTokens(draftTokens)})
            </span>
            {price ? (
              <span className="muted">
                ≈ {formatCost(estimate)} girdi · fiyat {formatUsd(price.input)} / {formatUsd(price.output)} (1M girdi/çıktı); çıktı yanıt uzunluğuna bağlı
              </span>
            ) : (
              <span className="muted">
                Bu modelin fiyatı bilinmiyor.{' '}
                <button type="button" className="link-btn" onClick={() => openG7Dialog({ kind: 'settings', section: 'costs' })}>
                  Fiyat tablosuna ekle
                </button>
              </span>
            )}
          </div>

          <div className="usage-section">
            <span className="usage-title">Bu görev</span>
            <span className="muted" role="status">
              {turn ? `${formatCost(turn.turnUsd)}${turn.live ? ' (akış sürüyor, tahmini)' : ''}` : 'Henüz çalışmadı'}
              {turn?.capUsd != null ? ` · tavan ${formatCost(turn.capUsd)}` : ''}
              {turn?.unpriced ? ' · fiyatı bilinmeyen model kullanıldı, tutar eksik olabilir' : ''}
            </span>
            {running && (
              <button type="button" className="btn small" onClick={onAbort}>
                Akışı kes
              </button>
            )}
          </div>

          <div className="usage-section">
            <span className="usage-title">Önbellek tasarrufu (bu oturum)</span>
            <span className="muted">
              {data && data.promptTokens > 0
                ? `${formatTokens(data.cachedTokens)} / ${formatTokens(data.promptTokens)} girdi token önbellekten (%${cachedPct})`
                : 'Henüz model çağrısı yok'}
              {savings !== null && savings > 0 ? ` · kazanç ≈ ${formatCost(savings)}` : ''}
            </span>
          </div>

          <div className="usage-section">
            <span className="usage-title">Oturum toplamı</span>
            <span className="muted">
              {formatCost(data?.sessionUsd ?? 0)}
              {data?.unpriced ? ' (fiyatı bilinmeyen modeller hariç)' : ''}
            </span>
          </div>
        </div>
      )}
    </div>
  )
}
