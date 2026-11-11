// Kor-to-Kor görünümü (§31.1, ADR 0057) için saf yardımcılar: yük metni, sıralama, yetki özeti,
// QR hücreleri ve yetki formu ayrıştırma. DOM yok; düğüm testleri çalıştırır.
import type { PeerGrant, PeerLoad, PeerRow } from '../../../ipc.gen.ts'

export function loadText(l: PeerLoad | null): string {
  if (!l) return 'ulaşılamıyor'
  const busy = l.busy == null ? 'yük bilinmiyor' : `%${Math.round(l.busy * 100)} dolu`
  return `${l.tasks} görev · ${l.cpus} çekirdek · ${busy}`
}

/** En boş makine önce; ulaşılamayanlar en sonda, adlarına göre. */
export function sortPeers(rows: PeerRow[]): PeerRow[] {
  return rows.slice().sort((a, b) => {
    if (a.score == null && b.score == null) return a.name.localeCompare(b.name)
    if (a.score == null) return 1
    if (b.score == null) return -1
    return a.score - b.score || a.name.localeCompare(b.name)
  })
}

export const READ_ONLY: PeerGrant = { dirs: [], write: false, commands: [], tools: [], memory: false }

export function grantText(g: PeerGrant): string {
  const parts: string[] = []
  if (g.dirs.length) parts.push(`${g.dirs.length} dizin${g.write ? ' (yazma)' : ' (okuma)'}`)
  if (g.commands.length) parts.push(`komut: ${g.commands.join(', ')}`)
  if (g.tools.length) parts.push(`araç: ${g.tools.join(', ')}`)
  if (g.memory) parts.push('hafıza')
  return parts.length ? parts.join(' · ') : 'salt-okunur'
}

/** Satır başına bir değer (boşlar atılır, tekrarlar tekilleşir). */
export function lines(text: string): string[] {
  return [...new Set(text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean))]
}

export interface GrantForm {
  dirs: string
  write: boolean
  commands: string
  tools: string
  memory: boolean
}

export const toForm = (g: PeerGrant): GrantForm => ({
  dirs: g.dirs.join('\n'),
  write: g.write,
  commands: g.commands.join('\n'),
  tools: g.tools.join('\n'),
  memory: g.memory,
})

export const fromForm = (f: GrantForm): PeerGrant => ({
  dirs: lines(f.dirs),
  write: f.write,
  commands: lines(f.commands),
  tools: lines(f.tools),
  memory: f.memory,
})

/** Koyu QR hücrelerinin koordinatları (SVG dikdörtgenleri için). */
export function qrCells(size: number, modules: boolean[]): [number, number][] {
  const out: [number, number][] = []
  for (let i = 0; i < Math.min(modules.length, size * size); i++) if (modules[i]) out.push([i % size, Math.floor(i / size)])
  return out
}
