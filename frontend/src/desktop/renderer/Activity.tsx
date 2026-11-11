import { ChevronRight } from 'lucide-react'
import { memo, useEffect, useState } from 'react'
import type { Approval } from '../../core/agent.ts'
import type { FileDecisions } from '../ipc-review.ts' // [review]
import { activityLabel, formatDuration, liveGroupLabel, summarizeTools } from './activity.ts'
import { MediaStrip } from './MediaStrip.tsx'
import { AgentCard } from './AgentCard.tsx'
import type { ChatItem, ToolItem } from './chat-state.ts'
import { ToolCard } from './ToolCard.tsx'
import { KorThinking } from './brand/brand.ts'
import { formatTokens, useAgentStatus } from './agent-status.ts'

function LineStats({ added, removed }: { added: number; removed: number }) {
  if (added === 0 && removed === 0) return null
  return (
    <span className="line-stats" aria-label={`${added} satır eklendi, ${removed} satır silindi`}>
      <span className="stat-add">+{added}</span>
      <span className="stat-del">−{removed}</span>
    </span>
  )
}

interface ActivityGroupProps {
  tools: ToolItem[]
  /** Alt ajanların araçlarını bulmak için sohbetin tüm öğeleri */
  allItems: readonly ChatItem[]
  onApprove: (id: string, approval: Approval, files?: FileDecisions) => void
  /** Tur sürüyor ve bu, akıştaki son grup: başlık canlı durum olur ("✻ Okunuyor dosya.ts 1dk 05sn ▾") ve liste açık gelir. */
  liveSince?: number | null
}

/**
 * Kor küpü (§16.1 KorThinking: glow, hot, spark, bob): ajan çalışırken sohbetteki tek durum göstergesi. Hareket
 * kapalıyken (OS isteği, Animasyonlar = Kapalı, düşük güç) CSS animasyonu durdurur; küp sönük kalır.
 */
function LiveCube({ size = 18 }: { size?: number }) {
  return (
    <span className="kor-live-cube" aria-hidden="true">
      <KorThinking size={size} label="" />
    </span>
  )
}

/**
 * Bitince: "2 dosya okundu, 1 dosya düzenlendi  +25 −6 ▸" — tıklayınca çerçeveli araç listesi açılır.
 * Çalışırken: "✻ Okunuyor dosya.ts  12sn ▾" ve liste açık. Alt ajanlar her zaman görünen kartlar olarak altta durur.
 */
export const ActivityGroup = memo(function ActivityGroup({ tools, allItems, onApprove, liveSince = null }: ActivityGroupProps) {
  const live = liveSince !== null
  // null: kullanıcı dokunmadı → canlıyken açık, bitince kapalı
  const [expanded, setExpanded] = useState<boolean | null>(null)
  const childrenOf = (id: string) => allItems.filter((i): i is ToolItem => i.kind === 'tool' && i.parentId === id)
  const agents = tools.filter((t) => t.name === 'agent')
  const others = tools.filter((t) => t.name !== 'agent')
  // Onay bekleyen bir araç varsa grup kapalı kalamaz; kullanıcı kararı görmeli.
  const needsApproval = others.some((t) => t.awaitingApproval)
  const open = (expanded ?? live) || needsApproval
  const all = [...tools, ...agents.flatMap((a) => childrenOf(a.id))]
  const added = all.reduce((n, t) => n + t.added, 0)
  const removed = all.reduce((n, t) => n + t.removed, 0)
  const label = live ? liveGroupLabel(tools, allItems) : summarizeTools(tools)

  return (
    <div className={`message activity${live ? ' live' : ''}`}>
      <button type="button" className="activity-head" onClick={() => setExpanded(!open)} aria-expanded={open} disabled={others.length === 0}>
        {live && <LiveCube size={16} />}
        <span key={live ? label : undefined} className={`activity-text${live ? ' label-swap' : ''}`}>
          {label}
        </span>
        {live && liveSince !== null && <span className="live-time">{formatDuration(Date.now() - liveSince)}</span>}
        <LineStats added={added} removed={removed} />
        {others.length > 0 && <ChevronRight size={14} className={`activity-chevron${open ? ' open' : ''}`} aria-hidden="true" />}
        {live && <span className="sr-only" aria-live="polite">{label}</span>}
      </button>
      {others.length > 0 && (
        <div className={`collapse${open ? ' open' : ''}`}>
          <div className="collapse-inner">
            <div className="activity-tools">
              {others.map((t) => (
                <ToolCard key={t.id} item={t} onApprove={onApprove} />
              ))}
            </div>
          </div>
        </div>
      )}
      {/* Üretilen görsel/videolar grup kapalıyken de görünür. */}
      <MediaStrip tools={others} />
      {agents.length > 0 && (
        <div className="agent-grid">
          {agents.map((a) => (
            <AgentCard key={a.id} tool={a} children={childrenOf(a.id)} onApprove={onApprove} />
          ))}
        </div>
      )}
    </div>
  )
})

/**
 * KorStatus: çalışırken alttaki satır, kor küpü + fiil + "1dk 05sn" + token. Fiil, süre ve token çekirdeğin
 * `agent.status` olayından gelir (§16.2): CLI spinner'ı ile aynı anda aynı sözcük. Küp yalnızca çizimdir.
 */
export function LiveStatus({ items, startedAt, sessionId }: { items: readonly ChatItem[]; startedAt: number; sessionId?: string }) {
  const status = useAgentStatus(sessionId, startedAt)
  const base = activityLabel(items)
  // Model düşünürken genel "Düşünüyor" yerine çekirdeğin fiili (Közlüyor, Dövüyor…); araç çalışırken araç fiili.
  const fromCore = base === 'Düşünüyor' && status !== null
  const label = fromCore ? status.verb : base
  return (
    <div className="message live-status" role="status" data-kor-event="kor:agent/status">
      <LiveCube />
      {/* key: fiil değişince yeni metin kayarak gelir */}
      <span key={label} className="live-label shimmer label-swap" translate={fromCore ? 'no' : undefined}>{label}…</span>
      {status !== null && <span className="live-time">{formatDuration(status.elapsedMs)}</span>}
      {status !== null && status.tokens > 0 && <span className="live-time">{formatTokens(status.tokens)}</span>}
      <span className="sr-only" aria-live="polite">{label}</span>
    </div>
  )
}

export function TurnEnd({ durationMs }: { durationMs: number }) {
  return (
    <div className="message turn-end">
      <span>{formatDuration(durationMs)} çalıştı</span>
    </div>
  )
}
