// Replay zaman çizelgesi mantığı (saf; node --test ile sınanır). Adım dizini `steps` içindedir.
import type { ReplayEvent, ReplayFile, ReplayStep } from '../../../ipc-replay.ts'

/** Konumu sınırlar içinde tutar. */
export function clamp(pos: number, len: number): number {
  return len <= 0 ? 0 : Math.min(Math.max(0, Math.trunc(pos)), len - 1)
}

/** Oynatırken bir sonraki adıma kadar beklenecek milisaniye (uzun aralar 2 sn ile sınırlı). */
export function delayMs(steps: readonly ReplayStep[], pos: number, speed: number): number {
  const a = steps[pos]
  const b = steps[pos + 1]
  if (!a || !b) return 0
  return Math.min(Math.max(0, b.t - a.t) / Math.max(0.1, speed), 2) * 1000
}

/** `t` saniyesindeki ya da ondan önceki son adım. */
export function seek(steps: readonly ReplayStep[], t: number): number {
  let at = 0
  steps.forEach((s, i) => {
    if (s.t <= t) at = i
  })
  return at
}

/** Bu adıma kadarki son diff olayı (geri gidince önceki diff görünür). */
export function diffAt(steps: readonly ReplayStep[], events: readonly ReplayEvent[], pos: number): ReplayEvent | null {
  for (let i = Math.min(pos, steps.length - 1); i >= 0; i--) {
    const s = steps[i]
    if (s?.kind === 'diff') return events[s.index] ?? null
  }
  return null
}

/** Diff olayının dosyaları, DiffView'e uygun biçimde (silme: sonrası boş metin). */
export function diffFiles(e: ReplayEvent | null): { path: string; before: string | null; after: string; truncated: boolean }[] {
  const files = Array.isArray(e?.files) ? (e.files as ReplayFile[]) : []
  return files
    .filter((f) => typeof f?.path === 'string')
    .map((f) => ({ path: f.path, before: f.before ?? null, after: f.after ?? '', truncated: f.truncated === true }))
}

/** Olayın ayrıntı alanları (metin olarak gösterilecek olanlar), türüne göre. */
export function detail(e: ReplayEvent | undefined): { label: string; text: string }[] {
  if (!e) return []
  const str = (k: string) => (typeof e[k] === 'string' ? (e[k] as string) : '')
  switch (e.kind) {
    case 'prompt':
      return [{ label: 'İstem', text: str('text') }, { label: 'Model', text: str('model') }]
    case 'text':
    case 'thinking':
    case 'notice':
      return [{ label: e.kind === 'thinking' ? 'Düşünme' : e.kind === 'notice' ? 'Bildirim' : 'Yanıt', text: str('text') }]
    case 'tool_start':
      return [{ label: 'Araç', text: `${str('name')} ${str('summary')}`.trim() }]
    case 'tool_end':
      return [{ label: e.is_error === true ? 'Araç hatası' : 'Araç çıktısı', text: str('output') }]
    case 'ask':
      return [{ label: 'Onay istendi', text: `${str('tool')} ${str('summary')}`.trim() }]
    case 'approval':
      return [{ label: 'Yanıt', text: str('reply') }]
    case 'usage': {
      const cost = typeof e.cost_usd === 'number' ? `${e.cost_estimated === true ? '≈' : ''}$${e.cost_usd.toFixed(4)}` : '—'
      return [{ label: 'Model', text: str('model') }, { label: 'Token', text: `${String(e.input)} / ${String(e.output)}` }, { label: 'Maliyet', text: cost }]
    }
    case 'end':
      return [{ label: 'Bitiş', text: str('error') || str('stop') }]
    default:
      return []
  }
}
