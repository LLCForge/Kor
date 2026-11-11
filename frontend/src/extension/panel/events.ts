// Kor olaylarını (masaüstü/CLI ile aynı `kor:event` gövdesi, desktop/ipc.ts AgentEventBody) yan panelde
// gösterilecek satırlara çevirir. Saf işlev: node --test ile sınanır.
import type { ConfirmEvent } from './Approval.tsx'

export type Item =
  | { kind: 'user'; text: string }
  | { kind: 'assistant'; text: string }
  | { kind: 'tool'; id: string; name: string; summary: string; done: boolean }
  | { kind: 'confirm'; ev: ConfirmEvent }
  | { kind: 'question'; id: string; question: string; options: string[] }
  | { kind: 'notice'; text: string }
  | { kind: 'error'; text: string }

export interface View {
  items: Item[]
  running: boolean
  /** KorStatus fiili ("Okuyor…") ve süresi. */
  verb: string
  /** Turun canlı maliyeti (g9:cost), metin. */
  cost: string
}

export const empty: View = { items: [], running: false, verb: '', cost: '' }

const str = (v: unknown) => (typeof v === 'string' ? v : '')

export function reduce(v: View, ev: Record<string, unknown>): View {
  const items = [...v.items]
  const last = items[items.length - 1]
  switch (ev.type) {
    case 'turnStart':
      return { ...v, running: true }
    case 'text': {
      if (last?.kind === 'assistant') items[items.length - 1] = { kind: 'assistant', text: last.text + str(ev.delta) }
      else items.push({ kind: 'assistant', text: str(ev.delta) })
      return { ...v, items }
    }
    case 'tool':
      if (items.some((i) => i.kind === 'tool' && i.id === ev.id)) return v
      items.push({ kind: 'tool', id: str(ev.id), name: str(ev.name), summary: str(ev.summary), done: false })
      return { ...v, items }
    case 'toolResult':
      return { ...v, items: items.map((i) => (i.kind === 'tool' && i.id === ev.id ? { ...i, done: true } : i)) }
    case 'confirm':
      items.push({
        kind: 'confirm',
        ev: {
          id: str(ev.id),
          name: str(ev.name),
          summary: str(ev.summary),
          warning: str(ev.warning) || undefined,
          destructive: typeof ev.destructive === 'string' ? ev.destructive : null,
          files: Array.isArray(ev.files) ? (ev.files as ConfirmEvent['files']) : [],
        },
      })
      return { ...v, items }
    case 'question':
      items.push({ kind: 'question', id: str(ev.id), question: str(ev.question), options: Array.isArray(ev.options) ? ev.options.map(String) : [] })
      return { ...v, items }
    case 'notice':
      items.push({ kind: 'notice', text: str(ev.text) })
      return { ...v, items }
    case 'error':
      items.push({ kind: 'error', text: str(ev.message) || 'hata' })
      return { ...v, items, running: false, verb: '' }
    case 'done':
      return { ...v, items: items.filter((i) => i.kind !== 'confirm' && i.kind !== 'question'), running: false, verb: '' }
    case 'g2:queueConsumed':
      for (const q of Array.isArray(ev.items) ? (ev.items as Array<{ text?: string }>) : []) items.push({ kind: 'user', text: str(q.text) })
      return { ...v, items }
    case 'agentStatus': {
      const s = (ev.status ?? {}) as Record<string, unknown>
      const secs = Math.round(Number(s.elapsedMs ?? 0) / 1000)
      return { ...v, running: true, verb: `${str(s.verb)} ${secs} sn` }
    }
    case 'g9:cost': {
      const usd = Number(ev.turnUsd ?? NaN)
      const estimated = ev.live === true || ev.unpriced === true
      return Number.isFinite(usd) ? { ...v, cost: `${estimated ? '≈ ' : ''}$${usd.toFixed(4)}` } : v
    }
    default:
      return v
  }
}

/** Bir onay/soru yanıtlandı: kartı kaldır. */
export function answered(v: View, id: string): View {
  return { ...v, items: v.items.filter((i) => !((i.kind === 'confirm' && i.ev.id === id) || (i.kind === 'question' && i.id === id))) }
}

/** Kayıtlı oturum dökümünden (`store::transcript`: `{ kind: user|assistant|tool, … }` dizisi) satırlar. */
export function fromTranscript(t: unknown): Item[] {
  const out: Item[] = []
  for (const m of (Array.isArray(t) ? t : []) as Array<Record<string, unknown>>) {
    if (m.kind === 'user' || m.kind === 'assistant') out.push({ kind: m.kind, text: str(m.text) })
    else if (m.kind === 'tool') out.push({ kind: 'tool', id: str(m.id), name: str(m.name), summary: str(m.summary), done: true })
  }
  return out
}
