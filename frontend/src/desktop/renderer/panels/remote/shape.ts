// Uzak oturum olaylarını satırlara indirger (DOM yok; düğüm testleri çalıştırır).
import type { RemoteTask, RemoteWireEvent } from '../../../ipc-remote.ts'

export type Row =
  | { kind: 'text'; text: string }
  | { kind: 'tool'; id: string; name: string; summary: string; result?: string }
  | { kind: 'notice' | 'error'; text: string }
  | { kind: 'confirm'; id: string; name: string; summary: string; danger: boolean; answer: string | null }

export interface Session {
  task: string | null
  running: boolean
  rows: Row[]
  tasks: RemoteTask[]
  /** Yeniden bağlanınca tekrar oynatılan olaylar iki kez işlenmesin. */
  lastSeq: number
}

export const EMPTY: Session = { task: null, running: false, rows: [], tasks: [], lastSeq: -1 }

const str = (v: unknown): string => (typeof v === 'string' ? v : '')

export function reduce(s: Session, ev: RemoteWireEvent): Session {
  if (ev.type === 'tasks') return { ...s, tasks: Array.isArray(ev.tasks) ? (ev.tasks as RemoteTask[]) : [] }
  if (ev.type === 'attached') {
    const task = str(ev.task)
    // Başka göreve geçiş: döküm o görevin tekrarıyla baştan kurulur.
    return task === s.task ? s : { ...s, task, rows: [], lastSeq: -1, running: false }
  }
  if (typeof ev.seq === 'number') {
    if (ev.seq <= s.lastSeq) return s
    s = { ...s, lastSeq: ev.seq }
  }
  const rows = s.rows.slice()
  const last = rows[rows.length - 1]
  switch (ev.type) {
    case 'turnStart':
      return { ...s, running: true }
    case 'done':
      return { ...s, running: false }
    case 'text':
      if (last?.kind === 'text') rows[rows.length - 1] = { kind: 'text', text: last.text + str(ev.delta) }
      else rows.push({ kind: 'text', text: str(ev.delta) })
      break
    case 'tool':
      if (!rows.some((r) => r.kind === 'tool' && r.id === ev.id)) rows.push({ kind: 'tool', id: str(ev.id), name: str(ev.name), summary: str(ev.summary) })
      break
    case 'toolResult': {
      const i = rows.findIndex((r) => r.kind === 'tool' && r.id === ev.id)
      if (i >= 0) rows[i] = { ...(rows[i] as Extract<Row, { kind: 'tool' }>), result: str(ev.result) }
      break
    }
    case 'confirm':
      rows.push({ kind: 'confirm', id: str(ev.id), name: str(ev.name), summary: str(ev.summary), danger: typeof ev.destructive === 'string', answer: null })
      break
    case 'confirmed': {
      const i = rows.findIndex((r) => r.kind === 'confirm' && r.id === ev.id)
      if (i >= 0) rows[i] = { ...(rows[i] as Extract<Row, { kind: 'confirm' }>), answer: str(ev.approval) }
      break
    }
    case 'notice':
      rows.push({ kind: 'notice', text: str(ev.text) })
      break
    case 'error':
      rows.push({ kind: 'error', text: str(ev.message) })
      break
    default:
      return s
  }
  return { ...s, rows }
}

/** Bekleyen (yanıtlanmamış) onay kartı. */
export function pendingConfirm(s: Session): Extract<Row, { kind: 'confirm' }> | null {
  for (let i = s.rows.length - 1; i >= 0; i--) {
    const r = s.rows[i]
    if (r?.kind === 'confirm' && r.answer === null) return r
  }
  return null
}

export function taskStateText(state: string): string {
  switch (state) {
    case 'queued':
      return 'sırada'
    case 'running':
      return 'çalışıyor'
    case 'waiting':
      return 'onay bekliyor'
    case 'idle':
      return 'bitti'
    case 'failed':
      return 'başarısız'
    default:
      return state
  }
}

export function memText(m: { total_mb: number; used_mb: number; available_mb: number } | null): string {
  if (!m) return '—'
  return `${m.used_mb} / ${m.total_mb} MB (${m.available_mb} MB boş)`
}
