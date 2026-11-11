import { useClock } from './clock.ts'
// [g3] Akış kartları: düşünme bloğu, model notu, değişiklik özeti, bölüm ayracı, sonraki adım, seçenekli soru, plan onayı, görev listesi.
import { Brain, Check, ChevronDown, ChevronRight, Circle, CircleCheck, CircleDot, ClipboardList, FileDiff, FilePlus, LoaderCircle, MessageCircleQuestion, PanelRight, PenLine, Sparkles, X } from 'lucide-react'
import { useEffect, useState, type FormEvent } from 'react'
import type { FormAsk, InteractAnswer, SimpleAsk, SuggestArgs, TodoItem } from '../../core/interactive.ts'
import { decodeInteractive, isAskForm, todoProgress } from '../../core/interactive.ts'
import { formatDuration } from './activity.ts'
import { errorMessage } from './api.ts'
import type { ChatItem, ToolItem } from './chat-state.ts'
import { g3Actions, useG3Actions } from './g3-bridge.ts'
import { chapterAnchor, isInLastTurn, type G3ChatItem } from './g3-state.ts'
import { Markdown } from './Markdown.tsx'
import { AssistantCard } from './AssistantCards.tsx' // [g23]
import { ASSISTANT_CARD_TOOLS } from './g23-cards.ts'

type Of<K extends G3ChatItem['kind']> = Extract<G3ChatItem, { kind: K }>

function useNow(active: boolean): number {
  const [start] = useState(() => Date.now())
  const tick = useClock(active, 1000) // [perf] paylaşımlı saat
  return active ? Math.max(tick, start) : start
}

/** "Düşündü · 12sn" — katlanabilir; düşünme sürerken canlı sayaç. */
export function ThinkingBlock({ item, live }: { item: Of<'thinking'>; live: boolean }) {
  const [open, setOpen] = useState(false)
  const now = useNow(live)
  const duration = (live ? now : item.endedAt) - item.startedAt
  return (
    <div className="message g3-thinking">
      <button type="button" className="g3-thinking-head" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Brain size={14} aria-hidden="true" />
        <span className={live ? 'shimmer' : undefined}>{live ? 'Düşünüyor' : 'Düşündü'}</span>
        <span className="g3-thinking-time">· {formatDuration(duration)}</span>
        <ChevronRight size={14} className={`activity-chevron${open ? ' open' : ''}`} aria-hidden="true" />
      </button>
      <div className={`collapse${open ? ' open' : ''}`}>
        <div className="collapse-inner">
          <p className="g3-thinking-text">{item.text.trim()}</p>
        </div>
      </div>
    </div>
  )
}

export function ModelNote({ item }: { item: Of<'modelNote'> }) {
  return (
    <div className="message g3-divider" role="note">
      <span>
        Model: <code>{item.from}</code> → <code>{item.to}</code>
      </span>
    </div>
  )
}

/** Turda değişen dosyalar; tıklayınca sağdaki fark paneli açılır. */
const CHANGE_CARD_KEY = 'cdx.changeCard.collapsed'
function readCollapsedPref(): boolean {
  try {
    return localStorage.getItem(CHANGE_CARD_KEY) === '1'
  } catch {
    return false
  }
}
export function ChangeSummaryCard({ item }: { item: Of<'changeSummary'> }) {
  const added = item.files.reduce((n, f) => n + f.added, 0)
  const removed = item.files.reduce((n, f) => n + f.removed, 0)
  // [cdx] Kart katlanabilir (tercih hatırlanır) ve kapatılabilir (yalnızca bu kart).
  const [collapsed, setCollapsed] = useState(readCollapsedPref)
  const [hidden, setHidden] = useState(false)
  const toggle = () => {
    const next = !collapsed
    setCollapsed(next)
    try {
      localStorage.setItem(CHANGE_CARD_KEY, next ? '1' : '0')
    } catch {
      /* tercih bu oturumla sınırlı */
    }
  }
  if (hidden) return null
  return (
    <div className="message">
      <section className={`g3-card g3-changes${collapsed ? ' collapsed' : ''}`} aria-label="Bu turdaki değişiklikler">
        <header className="g3-card-head cdx-changes-head">
          <button type="button" className="cdx-changes-toggle" onClick={toggle} aria-expanded={!collapsed} title={collapsed ? 'Listeyi aç' : 'Listeyi küçült'}>
            <ChevronDown size={14} className={`cdx-chevron${collapsed ? ' closed' : ''}`} aria-hidden="true" />
            <FileDiff size={14} aria-hidden="true" />
            <span>{item.files.length} dosya değişti</span>
          </button>
          <span className="line-stats">
            <span className="stat-add">+{added}</span>
            <span className="stat-del">−{removed}</span>
          </span>
          <button type="button" className="action-btn cdx-changes-open" onClick={() => g3Actions.current?.openChanges()} aria-label="Değişiklik panelini aç" title="Değişiklik panelini aç"><PanelRight size={13} aria-hidden="true" /></button>
          <button type="button" className="action-btn cdx-changes-close" onClick={() => setHidden(true)} aria-label="Kartı kapat" title="Kartı kapat"><X size={13} aria-hidden="true" /></button>
        </header>
        {!collapsed && (
        <ul className="g3-change-list">
          {item.files.map((f) => (
            <li key={f.path}>
              <button type="button" className="g3-change-row" onClick={() => g3Actions.current?.openChanges()} title="Farkları göster">
                {f.created ? <FilePlus size={13} aria-label="yeni dosya" /> : <FileDiff size={13} aria-hidden="true" />}
                <span className="truncate">{f.path}</span>
                <span className="line-stats">
                  <span className="stat-add">+{f.added}</span>
                  <span className="stat-del">−{f.removed}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
        )}
      </section>
    </div>
  )
}

function ChapterDivider({ id, title, summary }: { id: string; title: string; summary: string | undefined }) {
  return (
    <div className="message g3-chapter" id={chapterAnchor(id)}>
      <h2 className="g3-chapter-title text-balance">{title}</h2>
      {summary && <p className="g3-chapter-summary text-pretty">{summary}</p>}
    </div>
  )
}

function SuggestCard({ args, active }: { args: SuggestArgs; active: boolean }) {
  const actions = useG3Actions()
  return (
    <div className="message">
      <section className="g3-card g3-suggest" aria-label="Sonraki adım">
        <div className="g3-suggest-recap prose">
          <Markdown text={args.recap} />
        </div>
        {args.question && <p className="g3-suggest-question">{args.question}</p>}
        {args.options && args.options.length > 0 && (
          <div className="g3-actions">
            {args.options.map((o, i) => (
              <button
                key={o.label}
                type="button"
                className={`btn small${i === 0 ? ' primary' : ''}`}
                disabled={!active}
                onClick={() => actions()?.send(o.message)}
                title={o.message}
              >
                {o.label}
              </button>
            ))}
          </div>
        )}
      </section>
    </div>
  )
}

function useAnswer(callId: string) {
  const getActions = useG3Actions()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const submit = async (answer: InteractAnswer) => {
    const actions = getActions()
    if (!actions) return
    setBusy(true)
    setError(null)
    try {
      await actions.answer(callId, answer)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return { busy, error, submit }
}

function AskCard({ tool, args, waiting }: { tool: ToolItem; args: SimpleAsk; waiting: boolean }) {
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set())
  const [other, setOther] = useState('')
  const { busy, error, submit } = useAnswer(tool.id)
  const multi = args.multiSelect === true
  const disabled = !waiting || busy

  const choose = (label: string) => {
    if (!multi) {
      void submit({ kind: 'ask', selected: [label] })
      return
    }
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(label)) next.delete(label)
      else next.add(label)
      return next
    })
  }

  const send = (e: FormEvent) => {
    e.preventDefault()
    const text = other.trim()
    if (selected.size === 0 && !text) return
    void submit({ kind: 'ask', selected: [...selected], ...(text ? { other: text } : {}) })
  }

  // Claude Code kısayolu: 1..9 tuşu ilgili seçeneği seçer (yazı alanındayken çalışmaz).
  useEffect(() => {
    if (!waiting) return
    const onKey = (e: KeyboardEvent) => {
      if (e.altKey || e.ctrlKey || e.metaKey) return
      if ((e.target as HTMLElement | null)?.tagName === 'INPUT' || (e.target as HTMLElement | null)?.tagName === 'TEXTAREA') return
      const n = Number(e.key)
      if (!Number.isInteger(n) || n < 1 || n > args.options.length) return
      const option = args.options[n - 1]
      if (option) choose(option.label)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  return (
    <div className="message">
      <section className={`g3-card g3-ask${waiting ? ' waiting' : ''}`} aria-label="Ajanın sorusu">
        <header className="g3-card-head">
          <MessageCircleQuestion size={14} aria-hidden="true" />
          <span>{waiting ? 'Cevabın bekleniyor' : 'Soru'}</span>
          {multi && <span className="badge neutral">Birden çok seçilebilir</span>}
        </header>
        <p className="g3-ask-question">{args.question}</p>
        <div className="g3-options" role={multi ? 'group' : undefined} aria-label="Seçenekler">
          {args.options.map((o, i) => (
            <button
              key={o.label}
              type="button"
              className={`g3-option${selected.has(o.label) ? ' selected' : ''}`}
              onClick={() => choose(o.label)}
              disabled={disabled}
              {...(multi ? { 'aria-pressed': selected.has(o.label) } : {})}
            >
              {/* Claude Code deseni: seçenekler numaralı; 1..9 tuşlarıyla da seçilir. */}
              <span className="g3-num" aria-hidden="true">{i + 1}.</span>
              {multi && <span className="g3-check" aria-hidden="true">{selected.has(o.label) && <Check size={12} />}</span>}
              <span className="g3-option-text">
                <span className="g3-option-label">{o.label}</span>
                {o.description && <span className="g3-option-desc">{o.description}</span>}
              </span>
            </button>
          ))}
        </div>
        {waiting ? (
          <form className="g3-other" onSubmit={send}>
            <label htmlFor={`g3-other-${tool.id}`} className="sr-only">
              Diğer cevap
            </label>
            <input
              id={`g3-other-${tool.id}`}
              className="g3-input"
              value={other}
              onChange={(e) => setOther(e.target.value)}
              placeholder={multi ? 'Ek not veya kendi cevabın (isteğe bağlı)' : 'Diğer: kendi cevabını yaz'}
              maxLength={2000}
              disabled={busy}
            />
            <button type="submit" className="btn small primary" disabled={busy || (selected.size === 0 && !other.trim())}>
              {busy ? <LoaderCircle size={14} className="spin" aria-hidden="true" /> : null}
              Gönder
            </button>
          </form>
        ) : (
          tool.result !== null && <p className="g3-answer">{tool.result}</p>
        )}
        {error && (
          <p className="error-text g3-card-error" role="alert">
            {error}
          </p>
        )}
      </section>
    </div>
  )
}

function PlanCard({ tool, plan, waiting }: { tool: ToolItem; plan: string; waiting: boolean }) {
  const [mode, setMode] = useState<'auto' | 'default'>('auto')
  const [revising, setRevising] = useState(false)
  const [feedback, setFeedback] = useState('')
  const { busy, error, submit } = useAnswer(tool.id)
  const approved = tool.result?.startsWith('Kullanıcı planı onayladı') === true

  return (
    <div className="message">
      <section className={`g3-card g3-plan${waiting ? ' waiting' : ''}`} aria-label="Plan">
        <header className="g3-card-head">
          <ClipboardList size={14} aria-hidden="true" />
          <span>{waiting ? 'Plan onayın bekleniyor' : approved ? 'Plan onaylandı' : 'Plan'}</span>
        </header>
        <div className="prose g3-plan-body">
          <Markdown text={plan} />
        </div>
        {waiting && !revising && (
          <div className="g3-plan-actions">
            <fieldset className="g3-segmented" disabled={busy}>
              <legend className="sr-only">Onaydan sonraki izin modu</legend>
              {(['auto', 'default'] as const).map((m) => (
                <label key={m} className={mode === m ? 'active' : undefined}>
                  <input type="radio" name={`g3-mode-${tool.id}`} value={m} checked={mode === m} onChange={() => setMode(m)} />
                  {m === 'auto' ? 'Otomatik' : 'Manuel'}
                </label>
              ))}
            </fieldset>
            <button type="button" className="btn small primary" disabled={busy} onClick={() => void submit({ kind: 'planApprove', mode })}>
              {busy ? <LoaderCircle size={14} className="spin" aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}
              Onayla ve uygula
            </button>
            <button type="button" className="btn small" disabled={busy} onClick={() => setRevising(true)}>
              <PenLine size={14} aria-hidden="true" /> Planı düzenle
            </button>
          </div>
        )}
        {waiting && revising && (
          <form
            className="g3-revise"
            onSubmit={(e) => {
              e.preventDefault()
              if (feedback.trim()) void submit({ kind: 'planRevise', feedback: feedback.trim() })
            }}
          >
            <label htmlFor={`g3-feedback-${tool.id}`} className="g3-label">
              Planda neyi değiştirmeli?
            </label>
            <textarea
              id={`g3-feedback-${tool.id}`}
              className="g3-input"
              rows={3}
              value={feedback}
              onChange={(e) => setFeedback(e.target.value)}
              maxLength={5000}
              disabled={busy}
            />
            <div className="g3-actions">
              <button type="submit" className="btn small primary" disabled={busy || !feedback.trim()}>
                Geri bildirimi gönder
              </button>
              <button type="button" className="btn small ghost" disabled={busy} onClick={() => setRevising(false)}>
                Vazgeç
              </button>
            </div>
          </form>
        )}
        {!waiting && tool.result !== null && !approved && <p className="g3-answer">{tool.result}</p>}
        {error && (
          <p className="error-text g3-card-error" role="alert">
            {error}
          </p>
        )}
      </section>
    </div>
  )
}

function AskFormCard({ tool, args, waiting }: { tool: ToolItem; args: FormAsk; waiting: boolean }) {
  const [index, setIndex] = useState(0)
  const [selected, setSelected] = useState<string[]>([])
  const [ranked, setRanked] = useState<string[]>([])
  const [other, setOther] = useState('')
  const [rows, setRows] = useState<{ selected: string[]; other?: string; ranked?: string[]; skipped?: boolean }[]>([])
  const { busy, error, submit } = useAnswer(tool.id)
  const question = args.questions[index]
  if (!question) return null
  const disabled = !waiting || busy

  const remember = (row: { selected: string[]; other?: string; ranked?: string[]; skipped?: boolean }) => {
    const next = [...rows]
    next[index] = row
    setRows(next)
    return next
  }

  const go = (row: { selected: string[]; other?: string; ranked?: string[]; skipped?: boolean }) => {
    const next = remember(row)
    setSelected([])
    setRanked([])
    setOther('')
    if (index < args.questions.length - 1) {
      setIndex(index + 1)
      return
    }
    void submit({ kind: 'ask', selected: [], answers: next })
  }

  const move = (from: number, dir: -1 | 1) => {
    const order = ranked.length > 0 ? [...ranked] : [...question.options]
    const to = from + dir
    if (to < 0 || to >= order.length) return
    const [item] = order.splice(from, 1)
    if (item) order.splice(to, 0, item)
    setRanked(order)
  }

  const order = ranked.length > 0 ? ranked : question.options
  return (
    <div className="message">
      <section className={`g3-card g3-ask${waiting ? ' waiting' : ''}`} aria-label="Ajanın soruları">
        <header className="g3-card-head">
          <MessageCircleQuestion size={14} aria-hidden="true" />
          <span>
            {index + 1} / {args.questions.length}
          </span>
          {question.type === 'multi_select' && <span className="badge neutral">{selected.length} seçili</span>}
        </header>
        <p className="g3-ask-question">{question.question}</p>
        {question.type === 'rank_priorities' ? (
          <ol>
            {order.map((label, i) => (
              <li key={label} className="g23-rank">
                <span>{label}</span>
                <span className="g23-row">
                  <button type="button" className="btn small" disabled={disabled || i === 0} onClick={() => move(i, -1)} aria-label="Yukarı">
                    Yukarı
                  </button>
                  <button type="button" className="btn small" disabled={disabled || i === order.length - 1} onClick={() => move(i, 1)} aria-label="Aşağı">
                    Aşağı
                  </button>
                </span>
              </li>
            ))}
          </ol>
        ) : (
          <div className="g3-options">
            {question.options.map((label) => (
              <button
                key={label}
                type="button"
                className={`g3-option${selected.includes(label) ? ' selected' : ''}`}
                disabled={disabled}
                aria-pressed={question.type === 'multi_select' ? selected.includes(label) : undefined}
                onClick={() => {
                  if (question.type === 'single_select') {
                    go({ selected: [label] })
                    return
                  }
                  setSelected((prev) => (prev.includes(label) ? prev.filter((x) => x !== label) : [...prev, label]))
                }}
              >
                {label}
              </button>
            ))}
          </div>
        )}
        {waiting && (
          <div className="g23-row">
            <input
              className="g3-input"
              value={other}
              onChange={(e) => setOther(e.target.value)}
              placeholder="Başka bir şey"
              maxLength={2000}
              disabled={busy}
              aria-label="Başka bir şey"
            />
            <button type="button" className="btn small" disabled={busy} onClick={() => go({ selected: [], skipped: true })}>
              Atla
            </button>
            <button
              type="button"
              className="btn small primary"
              disabled={busy}
              onClick={() =>
                go(
                  question.type === 'rank_priorities'
                    ? { selected: [], ranked: order }
                    : { selected, ...(other.trim() ? { other: other.trim() } : {}) },
                )
              }
            >
              {index < args.questions.length - 1 ? 'İleri' : 'Gönder'}
            </button>
          </div>
        )}
        {!waiting && tool.result !== null && <p className="g3-answer">{tool.result}</p>}
        {error && (
          <p className="error-text g3-card-error" role="alert">
            {error}
          </p>
        )}
      </section>
    </div>
  )
}

interface InteractiveToolProps {
  tool: ToolItem
  items: readonly ChatItem[]
  running: boolean
  /** Çözülemeyen (geçersiz argümanlı) araç normal kartla gösterilir */
  fallback: () => React.ReactNode
}

/** mark_chapter, suggest_next, ask_user, exit_plan_mode ve yerel asistan kartlarını çizer. */
export function InteractiveTool({ tool, items, running, fallback }: InteractiveToolProps) {
  const waiting = running && tool.result === null
  const payload = decodeInteractive(tool.name, tool.summary)
  if (payload && !tool.result?.startsWith('Hata:')) {
    switch (payload.tool) {
      case 'mark_chapter':
        return <ChapterDivider id={tool.id} title={payload.args.title} summary={payload.args.summary} />
      case 'suggest_next':
        return <SuggestCard args={payload.args} active={!running && isInLastTurn(items, tool.id)} />
      case 'ask_user':
        return isAskForm(payload.args) ? (
          <AskFormCard tool={tool} args={payload.args} waiting={waiting} />
        ) : (
          <AskCard tool={tool} args={payload.args} waiting={waiting} />
        )
      case 'exit_plan_mode':
        return <PlanCard tool={tool} plan={payload.args.plan} waiting={waiting} />
      case 'todo_write':
        return null
    }
  }
  if (ASSISTANT_CARD_TOOLS.has(tool.name) && !tool.result?.startsWith('Hata:')) {
    const card = AssistantCard({ tool, waiting })
    if (card) return card
  }
  return <>{fallback()}</>
}

function TodoIcon({ status, running }: { status: TodoItem['status']; running: boolean }) {
  if (status === 'completed') return <CircleCheck size={15} className="g3-todo-icon done" aria-label="Tamamlandı" />
  // §23.3 tek spinner: süren görev dönmez, ember renkli işaretle gösterilir (durum composer çizgisinde ve KorStatus'ta).
  if (status === 'in_progress') return <CircleDot size={15} className="g3-todo-icon active" aria-label={running ? 'Sürüyor' : 'Yarım kaldı'} />
  return <Circle size={15} className="g3-todo-icon pending" aria-label="Bekliyor" />
}

/** Son görev listesi: akışın sonunda canlı güncellenir; hepsi bitince katlanır. */
export function TodoPanel({ todos, running }: { todos: readonly TodoItem[]; running: boolean }) {
  const { done, total, active } = todoProgress(todos)
  const finished = done === total && !running
  // Tur bitti ama liste yarım: "sürüyor" gibi görünmesin.
  const title = finished ? 'Tüm görevler tamamlandı' : running ? (active?.content ?? 'Görevler') : 'Görevler yarım kaldı'
  const [expanded, setExpanded] = useState<boolean | null>(null)
  const open = expanded ?? !finished
  return (
    <div className="message">
      <section className="g3-card g3-todos" aria-label="Görev listesi">
        <button type="button" className="g3-card-head g3-todos-head" onClick={() => setExpanded(!open)} aria-expanded={open}>
          <Sparkles size={14} aria-hidden="true" />
          <span>{title}</span>
          <span className="g3-todo-count">
            {done}/{total}
          </span>
          <ChevronRight size={14} className={`activity-chevron${open ? ' open' : ''}`} aria-hidden="true" />
        </button>
        <div className={`collapse${open ? ' open' : ''}`}>
          <div className="collapse-inner">
            <ol className="g3-todo-list">
              {todos.map((t, i) => (
                <li key={`${i}-${t.content}`} className={`g3-todo ${t.status}`}>
                  <TodoIcon status={t.status} running={running} />
                  <span className="g3-todo-text">{t.content}</span>
                </li>
              ))}
            </ol>
          </div>
        </div>
      </section>
    </div>
  )
}
