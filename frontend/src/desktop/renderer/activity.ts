import { TOOL_FAILURE, type ChatItem, type ToolItem } from './chat-state.ts'
import { CARD_TOOLS, HIDDEN_TOOLS } from './g3-state.ts'
import { ASSISTANT_CARD_TOOLS } from './g23-cards.ts' // [g23]
import { durationIntl } from './ui-locale.ts'

export type Segment =
  | { kind: 'item'; item: Exclude<ChatItem, ToolItem> }
  /** card: tek araçlık, kendi kartıyla çizilen etkileşimli araç (bölüm, soru, plan, sonraki adım) */
  | { kind: 'tools'; id: string; tools: ToolItem[]; card?: true }

/** Art arda gelen araç çağrılarını tek bir açılır etkinlik grubunda toplar. */
export function groupItems(items: readonly ChatItem[]): Segment[] {
  const segments: Segment[] = []
  for (const item of items) {
    // Alt ajanın araçları kendi kartında gösterilir, ana akışta değil.
    if (item.kind === 'tool' && item.parentId) continue
    // Görev listesi akışta değil, altta sabit listede gösterilir.
    if (item.kind === 'tool' && HIDDEN_TOOLS.has(item.name)) continue
    if (item.kind === 'tool' && (CARD_TOOLS.has(item.name) || ASSISTANT_CARD_TOOLS.has(item.name))) {
      segments.push({ kind: 'tools', id: item.id, tools: [item], card: true })
      continue
    }
    if (item.kind === 'tool') {
      const last = segments.at(-1)
      if (last?.kind === 'tools' && !last.card) last.tools.push(item)
      else segments.push({ kind: 'tools', id: item.id, tools: [item] })
    } else {
      segments.push({ kind: 'item', item })
    }
  }
  return segments
}

/**
 * Akış sırasında akan son öğe dışındaki liste. Hiçbir öğe değişmediyse önceki dizi aynen döner (hesaplar her
 * token'da çalışmaz); araç sonucu veya onay gibi yerinde güncellemeler yeni dizi üretir ve hemen görünür.
 */
export function settledItems(prev: readonly ChatItem[], items: ChatItem[], streaming: boolean): ChatItem[] {
  if (!streaming) return items
  const count = items.length - 1
  if (prev.length === count && prev.every((item, i) => item === items[i])) return prev as ChatItem[]
  return items.slice(0, -1)
}

const EDIT_TOOLS = new Set(['write_file', 'edit_file', 'multi_edit'])

/** "2 dosya okundu, 1 dosya düzenlendi, 1 komut çalıştırıldı" */
export function summarizeTools(tools: readonly ToolItem[]): string {
  const count = (pred: (t: ToolItem) => boolean) => tools.filter(pred).length
  const editedFiles = new Set(tools.filter((t) => EDIT_TOOLS.has(t.name)).map((t) => t.summary.split(' (')[0])).size
  const parts: string[] = []
  const read = count((t) => t.name === 'read_file')
  const searched = count((t) => t.name === 'grep')
  const listed = count((t) => t.name === 'list_files')
  const commands = count((t) => t.name === 'bash')
  const remembered = count((t) => t.name === 'remember')
  const agents = count((t) => t.name === 'agent')
  const shells = count((t) => t.name === 'bash_output' || t.name === 'kill_shell')
  if (read) parts.push(`${read} dosya okundu`)
  if (searched) parts.push(`${searched} arama yapıldı`)
  if (listed) parts.push('dosyalar listelendi')
  if (editedFiles) parts.push(`${editedFiles} dosya düzenlendi`)
  if (commands) parts.push(`${commands} komut çalıştırıldı`)
  if (agents) parts.push(`${agents} alt ajan çalıştırıldı`)
  if (shells) parts.push(`${shells} arka plan komutu yönetildi`)
  if (remembered) parts.push(remembered === 1 ? 'hafızaya not eklendi' : `hafızaya ${remembered} not eklendi`)
  const skills = count((t) => t.name === 'use_skill')
  if (skills) parts.push(skills === 1 ? 'skill çalıştırıldı' : `${skills} skill çalıştırıldı`)
  const known = new Set(['read_file', 'grep', 'list_files', 'bash', 'remember', 'agent', 'bash_output', 'kill_shell', 'use_skill', ...EDIT_TOOLS])
  const other = count((t) => !known.has(t.name))
  if (other) parts.push(`${other} araç kullanıldı`)
  const text = parts.join(', ')
  return text.charAt(0).toLocaleUpperCase('tr') + text.slice(1)
}

/** Araç satırının sağındaki kısa sonuç: "2 dosyada 3 eşleşme", "120 satır", "14 öğe". Bilinmeyen araçta boş. */
export function toolMeta(item: Pick<ToolItem, 'name' | 'result'>): string {
  const r = item.result
  if (r === null || TOOL_FAILURE.test(r)) return ''
  const lines = r.split('\n').filter((l) => l.trim() && !/^\[.*sınırlandı\]$/.test(l))
  switch (item.name) {
    case 'grep': {
      if (r.startsWith('Eşleşme yok')) return 'eşleşme yok'
      const files = new Set(lines.map((l) => /^(.+?):\d+:/.exec(l)?.[1]).filter(Boolean))
      return files.size > 0 ? `${files.size} dosyada ${lines.length} eşleşme` : `${lines.length} sonuç`
    }
    case 'list_files':
      return r.startsWith('Eşleşen dosya yok') ? 'sonuç yok' : `${lines.length} öğe`
    case 'read_file':
      return `${r.split('\n').length} satır`
    default:
      return ''
  }
}

/** Canlı başlıktaki kısa konu: dosya araçlarında dosya adı, diğerlerinde özetin başı. */
export function toolSubject(item: Pick<ToolItem, 'name' | 'summary'>): string {
  const s = item.summary.replace(/\s+\((?:tümü|arka plan|\d+ (?:karakter|düzenleme))\)$/, '')
  if (['read_file', 'write_file', 'edit_file', 'multi_edit'].includes(item.name)) return s.split(/[\\/]/).pop() ?? s
  return s.length > 60 ? `${s.slice(0, 59)}…` : s
}

export const ACTIVE_VERBS: Record<string, string> = {
  read_file: 'Okunuyor',
  write_file: 'Yazılıyor',
  edit_file: 'Düzenleniyor',
  multi_edit: 'Düzenleniyor',
  list_files: 'Dosyalar aranıyor',
  grep: 'Aranıyor',
  bash: 'Komut çalıştırılıyor',
  remember: 'Hafızaya yazılıyor',
  agent: 'Alt ajan çalışıyor',
  use_skill: 'Skill yükleniyor',
  run_python: 'Python çalışıyor',
  powershell: 'Komut çalıştırılıyor',
  web_fetch: 'Sayfa getiriliyor',
  web_search: "Web'de aranıyor",
  tool_search: 'Araç aranıyor',
  apply_patch: 'Yama uygulanıyor',
  monitor_shell: 'Komut izleniyor',
  bash_output: 'Arka plan çıktısı okunuyor',
  kill_shell: 'Arka plan komutu durduruluyor',
  mark_chapter: 'Bölüm açılıyor',
  todo_write: 'Görev listesi güncelleniyor',
  suggest_next: 'Özetleniyor',
  ask_user: 'Cevabın bekleniyor',
  exit_plan_mode: 'Plan onayın bekleniyor',
}

/** Canlı etkinlik grubunun başlığı: "Okunuyor SidePanel.tsx" — çalışan araç yoksa genel durum fiili. */
export function liveGroupLabel(tools: readonly ToolItem[], items: readonly ChatItem[]): string {
  const current = tools.findLast((t) => t.result === null && t.name !== 'agent')
  if (!current) return activityLabel(items)
  if (current.awaitingApproval) return 'Onay bekleniyor'
  return `${ACTIVE_VERBS[current.name] ?? 'Çalışıyor'} ${toolSubject(current)}`.trim()
}

/** Durum satırındaki fiil: ajan şu an ne yapıyor. */
export function activityLabel(items: readonly ChatItem[]): string {
  if (items.some((i) => i.kind === 'tool' && i.awaitingApproval)) return 'Onay bekleniyor'
  // Ön plandaki alt ajanlar sürerken ana ajan onları bekler.
  const runningAgents = items.filter((i) => i.kind === 'tool' && i.name === 'agent' && !i.subagent?.background && i.result === null).length
  if (runningAgents > 0) return runningAgents === 1 ? 'Alt ajan çalışıyor' : `${runningAgents} alt ajan paralel çalışıyor`
  const last = items.findLast((i) => !(i.kind === 'tool' && i.parentId))
  if (last?.kind === 'tool') {
    if (last.awaitingApproval) return 'Onay bekleniyor'
    if (last.result === null) return ACTIVE_VERBS[last.name] ?? 'Çalışıyor'
    return 'Düşünüyor'
  }
  if (last?.kind === 'assistant') return 'Yazıyor'
  return 'Düşünüyor'
}

/** 9sn · 1dk 05sn · 1sa 02dk */
export function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  const intl = durationIntl(total)
  if (intl) return intl
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  if (h > 0) return `${h}sa ${String(m).padStart(2, '0')}dk`
  if (m > 0) return `${m}dk ${String(s).padStart(2, '0')}sn`
  return `${s}sn`
}
