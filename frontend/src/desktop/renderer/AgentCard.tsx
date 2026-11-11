import { useClock } from './clock.ts'
import { Bot, Check, ChevronRight, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { Approval } from '../../core/agent.ts'
import type { FileDecisions } from '../ipc-review.ts' // [review]
import { formatDuration } from './activity.ts'
import { TOOL_FAILURE, TOOL_STOPPED, type ToolItem } from './chat-state.ts'
import { KorDots } from './brand/brand.ts'
import { Markdown } from './Markdown.tsx'
import { ToolCard } from './ToolCard.tsx'

function useElapsed(running: boolean, startedAt: number | undefined): number {
  const [mountedAt] = useState(() => Date.now())
  const start = startedAt ?? mountedAt
  const tick = useClock(running, 1000) // [perf] paylaşımlı saat
  const now = running ? Math.max(tick, start) : Date.now()
  return now - start
}

interface AgentCardProps {
  tool: ToolItem
  children: ToolItem[]
  onApprove: (id: string, approval: Approval, files?: FileDecisions) => void
}

/** Claude Code'daki alt ajan kartı: tip, görev, canlı durum; açılınca araçları ve raporu. */
export function AgentCard({ tool, children, onApprove }: AgentCardProps) {
  const sub = tool.subagent
  const running = sub ? sub.status === 'running' : tool.result === null
  // Reddedilen veya engellenen çağrıda alt ajan hiç başlamaz; sonuç metni tek kanıttır.
  const stopped = sub?.status === 'stopped' || tool.result === TOOL_STOPPED
  const failed = stopped || sub?.status === 'failed' || (tool.result !== null && TOOL_FAILURE.test(tool.result))
  const needsApproval = children.some((c) => c.awaitingApproval)
  const [expanded, setExpanded] = useState(false)
  const open = expanded || needsApproval
  const elapsed = useElapsed(running, sub?.startedAt)
  const lastChild = children.at(-1)
  const added = children.reduce((n, c) => n + c.added, 0)
  const removed = children.reduce((n, c) => n + c.removed, 0)

  const status = needsApproval
    ? 'Onay bekliyor'
    : running
      ? lastChild
        ? `${lastChild.name} · ${lastChild.summary}`
        : 'Başlıyor…'
      : stopped
        ? 'Durduruldu'
        : failed
          ? 'Başarısız'
          : `Tamamlandı · ${children.length} araç`

  return (
    <div className={`agent-card${running ? ' running' : ''}${failed ? ' failed' : ''}`}>
      <button type="button" className="agent-head" onClick={() => setExpanded((e) => !e)} aria-expanded={open}>
        <span className="agent-icon" aria-hidden="true">
          {/* §16.2 kural 5: görev başına KorDots (dönen Lucide halkası değil) */}
          {running ? <KorDots width={18} /> : failed ? <X size={14} /> : <Check size={14} />}
        </span>
        <span className="agent-type">
          <Bot size={13} aria-hidden="true" /> {sub?.agentType ?? 'agent'}
        </span>
        <span className="agent-desc">{sub?.description ?? tool.summary}</span>
        {sub?.background && <span className="badge neutral">Arka plan</span>}
        {(added > 0 || removed > 0) && (
          <span className="line-stats">
            <span className="stat-add">+{added}</span>
            <span className="stat-del">−{removed}</span>
          </span>
        )}
        <ChevronRight size={14} className={`activity-chevron${open ? ' open' : ''}`} aria-hidden="true" />
      </button>
      <div className="agent-status">
        <span className="truncate">{status}</span>
        {running && <span className="live-time">{formatDuration(elapsed)}</span>}
      </div>

      <div className={`collapse${open ? ' open' : ''}`}>
        <div className="collapse-inner">
          <div className="agent-body">
            {failed && !sub?.report && tool.result && <p className="muted agent-empty">{tool.result}</p>}
            {children.length === 0 && !failed && <p className="muted agent-empty">Henüz araç kullanmadı.</p>}
            {children.map((c) => (
              <ToolCard key={c.id} item={c} onApprove={onApprove} />
            ))}
            {sub?.report && (
              <div className="agent-report">
                <p className="agent-report-title">Rapor</p>
                <div className="prose">
                  <Markdown text={sub.report} />
                </div>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
