// [g8] Yapılandırılmış içerik blokları: sekme/akordeon (#104), adım sihirbazı (#105), yapılacaklar (#106, yerel
//      durum), kart galerisi (#107), zaman tüneli (#108), karşılaştırma tablosu (#109). İçerik React ile metin
//      olarak çizilir (enjeksiyon yok); ağ ya da dış görsel yüklenmez.
import { CheckSquare, GitCompareArrows, LayoutGrid, ListChecks, ListOrdered, Milestone } from 'lucide-react'
import { useState } from 'react'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { BlockFrame, parseJsonBlock, RawFallback } from './rich-shared.tsx'
import { usePersistentState } from './use-persistent-state.ts'

/** Yerel saklama anahtarı için kısa, kararlı karma (djb2). */
function hashKey(text: string): string {
  let h = 5381
  for (let i = 0; i < text.length; i++) h = ((h << 5) + h + text.charCodeAt(i)) >>> 0
  return h.toString(36)
}

// --- Sekme / akordeon (#104) --------------------------------------------------

interface TabItem {
  label: string
  content: string
}
interface TabsSpec {
  accordion?: boolean
  tabs: TabItem[]
}

function TabsView({ spec }: { spec: TabsSpec }) {
  const tabs = spec.tabs.filter((t) => typeof t?.label === 'string')
  const [active, setActive] = useState(0)
  if (spec.accordion) {
    return (
      <BlockFrame icon={<ListOrdered size={14} aria-hidden="true" />} label="Bölümler" className="g8-accordion">
        {tabs.map((t, i) => (
          <details key={i} className="g8-acc-item">
            <summary>{t.label}</summary>
            <div className="g8-acc-body">{String(t.content ?? '')}</div>
          </details>
        ))}
      </BlockFrame>
    )
  }
  return (
    <BlockFrame icon={<ListOrdered size={14} aria-hidden="true" />} label="Sekmeler" className="g8-tabs">
      <div className="g8-tablist" role="tablist">
        {tabs.map((t, i) => (
          <button
            key={i}
            type="button"
            role="tab"
            aria-selected={i === active}
            className={`g8-tab${i === active ? ' active' : ''}`}
            onClick={() => setActive(i)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="g8-tabpanel" role="tabpanel">
        {String(tabs[active]?.content ?? '')}
      </div>
    </BlockFrame>
  )
}

function TabsBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<TabsSpec>(code)
  if (!parsed.ok || !Array.isArray(parsed.data?.tabs)) return <RawFallback code={code} note={parsed.ok ? 'tabs dizisi gerekli' : parsed.error} />
  return <TabsView spec={parsed.data} />
}

// --- Adım sihirbazı (#105) ----------------------------------------------------

interface WizardSpec {
  title?: string
  steps: { title?: string; content: string }[]
}

function WizardView({ spec }: { spec: WizardSpec }) {
  const steps = spec.steps.filter((s) => s && typeof s.content === 'string')
  const [i, setI] = useState(0)
  const step = steps[i]
  return (
    <BlockFrame icon={<ListOrdered size={14} aria-hidden="true" />} label={spec.title ?? 'Sihirbaz'} className="g8-wizard">
      <div className="g8-wizard-prog" aria-hidden="true">
        {steps.map((_, n) => (
          <span key={n} className={`g8-wizard-dot${n === i ? ' active' : n < i ? ' done' : ''}`} />
        ))}
      </div>
      <p className="g8-wizard-count" role="status" aria-live="polite">
        Adım {i + 1} / {steps.length}
      </p>
      {step?.title && <h4 className="g8-wizard-title">{step.title}</h4>}
      <div className="g8-wizard-body">{String(step?.content ?? '')}</div>
      <div className="g8-wizard-nav">
        <button type="button" className="action-btn" onClick={() => setI((n) => Math.max(0, n - 1))} disabled={i === 0}>
          Geri
        </button>
        <button type="button" className="action-btn" onClick={() => setI((n) => Math.min(steps.length - 1, n + 1))} disabled={i >= steps.length - 1}>
          İleri
        </button>
      </div>
    </BlockFrame>
  )
}

function WizardBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<WizardSpec>(code)
  if (!parsed.ok || !Array.isArray(parsed.data?.steps) || parsed.data.steps.length === 0)
    return <RawFallback code={code} note={parsed.ok ? 'steps dizisi gerekli' : parsed.error} />
  return <WizardView spec={parsed.data} />
}

// --- Yapılacaklar (#106, yerel durum #119) ------------------------------------

interface TodoSpec {
  id?: string
  title?: string
  items: string[]
}

function TodoView({ spec }: { spec: TodoSpec }) {
  const items = spec.items.filter((t) => typeof t === 'string')
  const key = `todo:${spec.id ?? hashKey(items.join('\u0001'))}`
  // Durum yalnızca bu cihazda saklanır (try/catch'li localStorage); paylaşılmaz, boş gelebilir.
  const [checked, setChecked] = usePersistentState<boolean[]>(key, items.map(() => false))
  const done = checked.filter(Boolean).length
  const toggle = (i: number) => setChecked((prev) => items.map((_, n) => (n === i ? !prev[n] : (prev[n] ?? false))))
  return (
    <BlockFrame icon={<ListChecks size={14} aria-hidden="true" />} label={`${spec.title ?? 'Yapılacaklar'} · ${done}/${items.length}`} className="g8-todo">
      <ul className="g8-todo-list">
        {items.map((item, i) => (
          <li key={i}>
            <label className={`g8-todo-item${checked[i] ? ' done' : ''}`}>
              <input type="checkbox" checked={checked[i] ?? false} onChange={() => toggle(i)} />
              <CheckSquare size={15} aria-hidden="true" />
              <span>{item}</span>
            </label>
          </li>
        ))}
      </ul>
    </BlockFrame>
  )
}

function TodoBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<TodoSpec>(code)
  if (!parsed.ok || !Array.isArray(parsed.data?.items)) return <RawFallback code={code} note={parsed.ok ? 'items dizisi gerekli' : parsed.error} />
  return <TodoView spec={parsed.data} />
}

// --- Kart galerisi (#107) -----------------------------------------------------

interface GallerySpec {
  cards: { title?: string; text?: string; badge?: string; meta?: string }[]
}

function GalleryBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<GallerySpec>(code)
  if (!parsed.ok || !Array.isArray(parsed.data?.cards)) return <RawFallback code={code} note={parsed.ok ? 'cards dizisi gerekli' : parsed.error} />
  const cards = parsed.data.cards
  return (
    <BlockFrame icon={<LayoutGrid size={14} aria-hidden="true" />} label="Galeri" className="g8-gallery">
      <div className="g8-grid">
        {cards.map((c, i) => (
          <article key={i} className="g8-card">
            {c.badge && <span className="g8-badge">{c.badge}</span>}
            {c.title && <h4 className="g8-card-title">{c.title}</h4>}
            {c.text && <p className="g8-card-text">{c.text}</p>}
            {c.meta && <p className="g8-card-meta">{c.meta}</p>}
          </article>
        ))}
      </div>
    </BlockFrame>
  )
}

// --- Zaman tüneli (#108) ------------------------------------------------------

interface TimelineSpec {
  events: { date?: string; title?: string; text?: string }[]
}

function TimelineBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<TimelineSpec>(code)
  if (!parsed.ok || !Array.isArray(parsed.data?.events)) return <RawFallback code={code} note={parsed.ok ? 'events dizisi gerekli' : parsed.error} />
  return (
    <BlockFrame icon={<Milestone size={14} aria-hidden="true" />} label="Zaman tüneli" className="g8-timeline">
      <ol className="g8-tl-list">
        {parsed.data.events.map((e, i) => (
          <li key={i} className="g8-tl-item">
            <span className="g8-tl-dot" aria-hidden="true" />
            <div className="g8-tl-content">
              {e.date && <span className="g8-tl-date">{e.date}</span>}
              {e.title && <h4 className="g8-tl-title">{e.title}</h4>}
              {e.text && <p className="g8-tl-text">{e.text}</p>}
            </div>
          </li>
        ))}
      </ol>
    </BlockFrame>
  )
}

// --- Karşılaştırma tablosu (#109) ---------------------------------------------

interface CompareSpec {
  a: string
  b: string
  rows: { label: string; a?: string; b?: string }[]
}

function CompareBlock({ code }: RichBlockProps) {
  const parsed = parseJsonBlock<CompareSpec>(code)
  if (!parsed.ok || !Array.isArray(parsed.data?.rows)) return <RawFallback code={code} note={parsed.ok ? 'rows dizisi gerekli' : parsed.error} />
  const { a, b, rows } = parsed.data
  return (
    <BlockFrame icon={<GitCompareArrows size={14} aria-hidden="true" />} label="Karşılaştırma" className="g8-compare">
      <table>
        <thead>
          <tr>
            <th scope="col" />
            <th scope="col">{a ?? 'A'}</th>
            <th scope="col">{b ?? 'B'}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => (
            <tr key={i}>
              <th scope="row">{r.label}</th>
              <td>{r.a ?? ''}</td>
              <td>{r.b ?? ''}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </BlockFrame>
  )
}

registerRichBlock('tabs', TabsBlock)
registerRichBlock('wizard', WizardBlock)
registerRichBlock('steps', WizardBlock)
registerRichBlock('todo', TodoBlock)
registerRichBlock('gallery', GalleryBlock)
registerRichBlock('cards', GalleryBlock)
registerRichBlock('timeline', TimelineBlock)
registerRichBlock('compare', CompareBlock)
