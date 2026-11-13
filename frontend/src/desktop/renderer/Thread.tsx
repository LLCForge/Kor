import { ArrowDown, Bell, RotateCw } from 'lucide-react'
import { Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { trackedMemo } from './perfmon/tracked.tsx'
import type { Approval } from '../../core/agent.ts'
import type { FileDecisions } from '../ipc-review.ts' // [review]
import { groupItems, settledItems, type Segment } from './activity.ts'
import { ActivityGroup, LiveStatus, TurnEnd } from './Activity.tsx'
import { ChapterToc } from './ChapterToc.tsx'
import { isTopLevel, type ChatItem } from './chat-state.ts'
import { CompactionDivider } from './CompactionDivider.tsx' // [g6]
import { rowsToFold } from './compaction-fold.ts' // [g6]
import { g3Actions, G3PaneContext, type G3PaneActions } from './g3-bridge.ts'
import { chaptersOf, isInLastTurn, lastUserMessage, latestTodos } from './g3-state.ts'
import { ChangeSummaryCard, InteractiveTool, ModelNote, ThinkingBlock, TodoPanel } from './InteractiveCards.tsx'
import { LinkPreviews } from './LinkPreviews.tsx'
import { Markdown } from './Markdown.tsx'
import { extractLinks } from './markdown-utils.ts'
import { MessageActions } from './MessageActions.tsx'
import { ThreadSearch } from './ThreadSearch.tsx'
import { useAutoScroll } from './use-auto-scroll.ts'
import { G2AssistantActions, G2UserActions, InlineEditBox } from './g2/MessageActions.tsx' // [g2]
import { useEditingId } from './edit-store.ts' // [cdx] satır içi düzenleme
import { G17MessageActions } from './G17MessageActions.tsx' // [g17]
import { ForkHere } from './fork/ForkHere.tsx' // §27.3
import { G17ThreadBar } from './G17ThreadBar.tsx' // [g17]
import { modelName, modelsByItem } from './message-model.ts'
import { ProviderLogo } from './panels/providers/ProviderLogo.tsx'
import { providerOf } from '../../core/providers.ts'
import { THREAD_WINDOW, windowed } from './perfmon/util.ts' // §32.2 uzun sohbet penceresi (ADR 0062)

// Boş durum / karşılama: HomeEmpty.tsx (§23.3, §23.4).

type UserItem = Extract<ChatItem, { kind: 'user' }>
type AssistantItem = Extract<ChatItem, { kind: 'assistant' }>

/** Bu yükseklikten uzun kullanıcı mesajı katlanır (alt kenarı solarak) ve "Daha fazla göster" çıkar. styles.css .user-text.clamped ile aynı. */
const USER_CLAMP_PX = 200

export function UserText({ text }: { text: string }) {
  const ref = useRef<HTMLDivElement>(null)
  const [overflows, setOverflows] = useState(false)
  const [expanded, setExpanded] = useState(false)
  // Boyamadan önce ölçülür: uzun mesaj bir an tam boy görünüp sonra katlanmasın.
  useLayoutEffect(() => {
    const el = ref.current
    if (el) setOverflows(el.scrollHeight > USER_CLAMP_PX + 40)
  }, [text])
  const clamped = overflows && !expanded
  return (
    <>
      <div ref={ref} className={`user-text${clamped ? ' clamped' : ''}`}>
        {text}
      </div>
      {overflows && (
        <button type="button" className="user-more" onClick={() => setExpanded((e) => !e)} aria-expanded={expanded}>
          {expanded ? 'Daha az göster' : 'Daha fazla göster'}
        </button>
      )}
    </>
  )
}

// [perf] Eylemler memo'nun içinde çizilir: JSX prop'u her çizimde yenilenip memo'yu boşa çıkarıyordu.
const UserMessage = trackedMemo('UserMessage', function UserMessage({ item, at, extraActions, items, running, inPane }: { item: UserItem; at: number | undefined; extraActions?: ReactNode; items: readonly ChatItem[]; running: boolean; inPane: boolean }) {
  const editing = useEditingId() === item.id
  if (editing) {
    return (
      <div className="message user editing" id={`g17-msg-${item.id}`}>
        <InlineEditBox item={item} items={items} />
      </div>
    )
  }
  return (
    <div className="message user" id={`g17-msg-${item.id}`}>
      <div className="user-bubble">
        {item.attachments.length > 0 && (
          <div className="bubble-files">
            {item.attachments.map((p) => <span key={p} className="file-pill">{p}</span>)}
          </div>
        )}
        <UserText text={item.text} />
      </div>
      {/* [g4] "Buraya geri sar" vb. ek düğmeler */}
      <MessageActions getText={() => item.text} at={at}>
        {!inPane && <G2UserActions item={item} items={items} running={running} />}
        {extraActions}
        {/* [g17] sabitle, seç, kalıcı bağlantı */}
        <G17MessageActions item={item} items={items} />
        {/* §27.3 buradan dallan */}
        {!inPane && <ForkHere item={item} items={items} />}
      </MessageActions>
    </div>
  )
})

/** "<logo> deepseek-v4-1-flash yanıtladı" (§23.2). */
function AnswerHeader({ model }: { model: string }) {
  const provider = providerOf(model)
  const name = modelName(model, provider)
  return (
    <div className="kor-answer-head">
      <ProviderLogo provider={provider} modelId={name} size={14} />
      <span translate="no">{name}</span>
      <span className="muted">yanıtladı</span>
    </div>
  )
}

const AssistantMessage = trackedMemo('AssistantMessage', function AssistantMessage({
  item,
  streaming,
  at,
  items,
  running,
  inPane,
  model,
}: {
  item: AssistantItem
  streaming: boolean
  at: number | undefined
  items: readonly ChatItem[]
  running: boolean
  inPane: boolean
  /** Bu yanıtı veren model (§23.2 mesaj başlığı); bilinmiyorsa başlık çizilmez */
  model?: string
}) {
  // Akış sürerken bağlantı listesi oynamasın diye yanıt bitince çıkarılır.
  const links = useMemo(() => (streaming ? [] : extractLinks(item.text)), [item.text, streaming])
  return (
    <div className="message assistant" id={`g17-msg-${item.id}`}>
      {model && <AnswerHeader model={model} />}
      {/* Akış sürerken son paragrafın sonunda yanıp sönen imleç */}
      <div className={`prose${streaming ? ' streaming' : ''}`}>
        <Markdown text={item.text} />
      </div>
      <LinkPreviews urls={links} />
      <MessageActions getText={() => item.text} at={at}>
        {/* [g2] yeniden üret */}
        {!inPane && <G2AssistantActions item={item} items={items} running={running} />}
        {/* [g17] dışa aktar, sabitle, seç, karşılaştır, kalıcı bağlantı */}
        <G17MessageActions item={item} items={items} />
        {/* §27.3 buradan dallan */}
        {!inPane && <ForkHere item={item} items={items} />}
      </MessageActions>
    </div>
  )
})

interface ThreadProps {
  items: ChatItem[]
  running: boolean
  turnStartedAt: number | null
  composer: ReactNode
  onApprove: (id: string, approval: Approval, files?: FileDecisions) => void
  /** [g4] kullanıcı mesajının eylem çubuğuna eklenen düğmeler ("Buraya geri sar") */
  renderUserActions?: (item: Extract<ChatItem, { kind: 'user' }>) => ReactNode
  /** [g6] düzenlenen sıkıştırma özetini kaydeder */
  onSaveSummary?: (foldId: string, body: string) => Promise<boolean>
  /** Yan bölme: kartlar ve yeniden dene bu sohbete bağlanır (g2 düzenle/yeniden üret ana sohbete bağlı olduğundan gizlenir) */
  pane?: G3PaneActions
  /** `agent.status` olaylarını bu oturuma bağlar (§16.2) */
  sessionId?: string
  /** Oturumun güncel modeli: yanıt başlıklarında logo + model adı (§23.2) */
  model?: string
}

export function Thread({ items, running, turnStartedAt, composer, onApprove, renderUserActions, onSaveSummary, pane, sessionId, model = '' }: ThreadProps) {
  const answerModels = useMemo(() => (model ? modelsByItem(items, model) : new Map<string, string>()), [items, model])
  const viewportRef = useRef<HTMLDivElement>(null)
  const messagesRef = useRef<HTMLDivElement>(null)
  const [searchOpen, setSearchOpen] = useState(false)
  const last = items.at(-1)
  const { atBottom, unseen, scrollToBottom } = useAutoScroll(viewportRef, messagesRef, last?.kind === 'user' ? last.id : undefined)
  // [g6] Sıkıştırma sonrası özetlenmiş eski bloklar DOM'a hiç girmez; aksi hâlde uzun sohbet ilk mesaja kadar
  // yüzlerce düğüm çizip donuyordu. Özet ayracı açılınca o bloklar yeniden erişilebilir olur.
  // Kullanıcı açtıysa ve yeni sıkıştırma gelmediyse seçim korunur (her olayda kapanmasın).
  const [revealBefore, setRevealBefore] = useState<string | null>(null)
  const foldKey = items.findLast((i) => i.kind === 'compaction')?.id ?? null
  const lastFold = useRef(foldKey)
  useEffect(() => {
    if (lastFold.current === foldKey) return
    lastFold.current = foldKey
    setRevealBefore(null)
  }, [foldKey])
  // [perf] Akış sırasında her token yeni bir items dizisi üretir; katlama/gruplama/bölüm hesapları ve tüm mesaj
  // bileşenleri her delta'da yeniden çalışmasın diye akan son asistan balonu "yerleşik" listeden ayrılır.
  const lastStreaming = running && last?.kind === 'assistant'
  const settledRef = useRef(items)
  const settled = settledItems(settledRef.current, items, lastStreaming)
  settledRef.current = settled
  // Alt ajan araçları sona eklense de akan öğe budur (imleç ve canlı düşünme için).
  const top = items.findLast(isTopLevel)
  const approveRef = useRef(onApprove)
  approveRef.current = onApprove
  const stableApprove = useCallback<ThreadProps['onApprove']>((id, approval, files) => approveRef.current(id, approval, files), [])
  const folds = useMemo(() => rowsToFold(settled, revealBefore), [settled, revealBefore])
  const settledSegments = useMemo(() => groupItems(folds.visible), [folds])
  const segments = useMemo<Segment[]>(() => (lastStreaming && last ? [...settledSegments, ...groupItems([last])] : settledSegments), [settledSegments, lastStreaming, last])
  // Tur sürerken akışın sonundaki (kart olmayan) araç grubu canlı başlık taşır; altta ayrı durum satırı çıkmaz.
  const tail = segments.at(-1)
  const liveSegment = running && turnStartedAt !== null && tail?.kind === 'tools' && !tail.card ? tail : null
  const groupStarts = useRef(new Map<string, number>())
  const groupSince = (id: string): number => {
    const known = groupStarts.current.get(id)
    if (known !== undefined) return known
    const now = Date.now()
    groupStarts.current.set(id, now)
    return now
  }
  // §32.2: 10k mesajlık sohbet açılışta yalnızca son THREAD_WINDOW bölümü çizer (sanallaştırma); üstteki gözcü
  // görünür olunca pencere büyür ve kaydırma konumu korunur. Arama ve bölüm atlama DOM'da çalıştığı için
  // onlar tamamını açar.
  const [shown, setShown] = useState(THREAD_WINDOW)
  useEffect(() => setShown(THREAD_WINDOW), [sessionId])
  useEffect(() => {
    if (searchOpen) setShown(Number.POSITIVE_INFINITY)
  }, [searchOpen])
  const { hidden: hiddenSegments, visible: shownSegments } = windowed(segments, shown)
  const moreRef = useRef<HTMLButtonElement>(null)
  const anchor = useRef<{ height: number; top: number } | null>(null)
  const showMore = useCallback(() => {
    const v = viewportRef.current
    if (v) anchor.current = { height: v.scrollHeight, top: v.scrollTop }
    setShown((s) => s + THREAD_WINDOW)
  }, [])
  useLayoutEffect(() => {
    const v = viewportRef.current
    const a = anchor.current
    if (!v || !a) return
    anchor.current = null
    v.scrollTop = a.top + (v.scrollHeight - a.height)
  }, [shown])
  useEffect(() => {
    const el = moreRef.current
    const root = viewportRef.current
    if (!el || !root || typeof IntersectionObserver === 'undefined') return
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) showMore()
    }, { root, rootMargin: '100% 0px 0px 0px' })
    io.observe(el)
    return () => io.disconnect()
  }, [hiddenSegments > 0, showMore])
  const chapters = useMemo(() => chaptersOf(settled), [settled])
  const todos = useMemo(() => latestTodos(settled, running), [settled, running])
  const lastError = items.findLast((i) => i.kind === 'error')
  // Yeniden dene yalnızca son turda kalan hatada; sonrasında yeni mesaj gönderildiyse gizlenir.
  const canRetry = !running && lastError !== undefined && isInLastTurn(items, lastError.id)
  const retryTarget = lastUserMessage(items)
  const actions = pane ?? g3Actions.current
  const times = actions?.times ?? {}

  // Ctrl+F (macOS'ta Cmd+F) sohbet içi aramayı açar; açıksa kutuya odaklanır.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'f') {
        e.preventDefault()
        setSearchOpen(true)
        document.getElementById('g3-search-input')?.focus()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // Arama DOM'da yürür; katlanmış bloklar DOM'da olmadığı için arama açılınca geçmiş bir kez açılır.
  useEffect(() => {
    if (searchOpen && folds.foldId) setRevealBefore(folds.foldId)
  }, [searchOpen, folds.foldId])

  return (
    <G3PaneContext.Provider value={pane ?? null}>
    <div className="g3-thread">
      {searchOpen && <ThreadSearch containerRef={messagesRef} version={items} onClose={() => setSearchOpen(false)} />}
      <ChapterToc chapters={chapters} scrollRef={viewportRef} onMissing={() => setShown(Number.POSITIVE_INFINITY)} />
      <div className="viewport" ref={viewportRef}>
        <div className="messages" ref={messagesRef}>
          {/* [g17] filtre, sabitlenenler, seçim/dışa aktarma, özet, karşılaştırma araç çubuğu */}
          <G17ThreadBar items={settled} containerRef={messagesRef} />
          {hiddenSegments > 0 && (
            <button ref={moreRef} type="button" className="btn small thread-more" onClick={showMore}>
              Daha eski mesajları göster
            </button>
          )}
          {shownSegments.map((segment) => {
            if (segment.kind === 'tools') {
              // Başlıktaki süre turun değil bu grubun süresi: yoksa uzun turda "Aranıyor x 1sa 36dk" görünüyordu.
              const live = segment === liveSegment ? groupSince(segment.id) : null
              const group = <ActivityGroup key={segment.id} tools={segment.tools} allItems={settled} onApprove={stableApprove} liveSince={live} />
              const tool = segment.tools[0]
              if (segment.card && tool) return <InteractiveTool key={segment.id} tool={tool} items={items} running={running} fallback={() => group} />
              // Sarmalayıcı hep aynı: görev listesi gelip gidince grup yeniden yüklenip kart durumunu kaybetmesin.
              return (
                <Fragment key={segment.id}>
                  {segment === liveSegment && todos && <TodoPanel todos={todos} running={running} />}
                  {group}
                </Fragment>
              )
            }
            const item = segment.item
            switch (item.kind) {
              case 'user':
                return (
                  <UserMessage
                    key={item.id}
                    item={item}
                    items={settled}
                    running={running}
                    inPane={pane !== undefined}
                    at={times[item.id]}
                    extraActions={renderUserActions?.(item)}
                  />
                )
              case 'assistant':
                return (
                  <AssistantMessage
                    key={item.id}
                    item={item}
                    streaming={running && item === top}
                    at={times[item.id]}
                    // Akan balon listede son öğedir; diğerleri her token'da değişmeyen listeyi alır.
                    items={item === last ? items : settled}
                    running={running}
                    inPane={pane !== undefined}
                    model={answerModels.get(item.id)}
                  />
                )
              case 'error':
                return (
                  <div key={item.id} className="message">
                    <div className="error-box g3-error" role="alert">
                      <span>{item.text}</span>
                      {item === lastError && canRetry && retryTarget && actions && (
                        <button
                          type="button"
                          className="btn small g3-retry"
                          onClick={() => actions.send(retryTarget.text, retryTarget.attachments)}
                        >
                          <RotateCw size={14} aria-hidden="true" /> Yeniden dene
                        </button>
                      )}
                    </div>
                  </div>
                )
              case 'notice':
                return (
                  <div key={item.id} className={`message notice${item.alert ? ' error-text' : ''}`} role={item.alert ? 'alert' : undefined}>
                    <Bell size={13} aria-hidden="true" />
                    <span>{item.text}</span>
                  </div>
                )
              case 'turnEnd':
                return <TurnEnd key={item.id} durationMs={item.durationMs} />
              case 'thinking':
                return <ThinkingBlock key={item.id} item={item} live={running && item === top} />
              case 'modelNote':
                return <ModelNote key={item.id} item={item} />
              case 'changeSummary':
                return <ChangeSummaryCard key={item.id} item={item} />
              // [g6] bağlam sıkıştırma ayracı: önceki bloklar katlanmışsa buradan açılır
              case 'compaction':
                return (
                  <CompactionDivider
                    key={item.id}
                    item={{
                      ...item,
                      // Düzenleme yalnızca en son özette (modelin gördüğü) anlamlı.
                      ...(item.id === folds.foldId && onSaveSummary ? { onSave: (body: string) => onSaveSummary(item.id, body) } : {}),
                    }}
                    hiddenCount={item.id === folds.foldId ? folds.hiddenCount : 0}
                    revealed={folds.foldId === item.id && folds.hiddenCount === 0}
                    onToggle={() => {
                      const opening = revealBefore !== item.id
                      setRevealBefore(opening ? item.id : null)
                      // Açarken en eski bloğa değil, ayraca yerleş: kullanıcı nereden devam ettiğini görsün.
                      if (opening) requestAnimationFrame(() => document.getElementById(`g6-fold-${item.id}`)?.scrollIntoView({ block: 'center', behavior: 'smooth' }))
                    }}
                  />
                )
            }
          })}
          {todos && !liveSegment && <TodoPanel todos={todos} running={running} />}
          {/* Canlı düşünme bloğu ve canlı araç grubu zaten "fiil · süre" gösteriyor; ikinci bir durum satırı çıkmasın. */}
          {running && turnStartedAt !== null && top?.kind !== 'thinking' && !liveSegment && <LiveStatus items={items} startedAt={turnStartedAt} sessionId={sessionId} />}
        </div>

        <div className="viewport-footer">
          {!atBottom && (
            <button type="button" className="g3-jump pop-in" onClick={scrollToBottom} aria-label={unseen ? 'En alta git (yeni içerik var)' : 'En alta git'} title="En alta git">
              <ArrowDown size={16} aria-hidden="true" />
              {unseen && <span className="g3-jump-dot" aria-hidden="true" />}
            </button>
          )}
          {composer}
          <p className="disclaimer">Kor hata yapabilir. Yanıtları ve değişiklikleri kontrol et.</p>
        </div>
      </div>
    </div>
    </G3PaneContext.Provider>
  )
}

