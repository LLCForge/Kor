// Proje grafı panelinin saf yardımcıları (filtre, vurgu katmanları, metinler). DOM yok; düğüm testleri çalıştırır.
import type { EdgeKind, Explanation, NodeKind, NodeRef, PathStep, Snapshot, SnapshotNode } from '../../../ipc-graph.ts'
import { uiLocale, agoIntl } from '../../ui-locale.ts'

export const KIND_LABEL: Record<NodeKind, string> = {
  function: 'Fonksiyon',
  struct: 'Yapı / sınıf',
  trait: 'Arayüz',
  module: 'Dosya',
  table: 'Tablo',
  env_var: 'Ortam değişkeni',
  config: 'Yapılandırma',
  doc: 'Belge',
  decision: 'Karar',
  package: 'Paket',
  server: 'Sunucu',
  service: 'Servis',
}

export const ALL_KINDS = Object.keys(KIND_LABEL) as NodeKind[]

export const EDGE_LABEL: Record<EdgeKind['kind'], string> = {
  calls: 'çağırır',
  imports: 'içe aktarır',
  implements: 'uygular',
  extends: 'genişletir',
  contains: 'içerir',
  reads_table: 'tablo okur',
  writes_table: 'tabloya yazar',
  reads_env: 'ortam okur',
  co_changes: 'birlikte değişir',
  documented_by: 'belgelenir',
  decided_by: 'karar',
  depends_on: 'bağımlı',
  references: 'başvurur',
  runs_on: 'üzerinde çalışır',
}

export interface Filter {
  kinds: ReadonlySet<NodeKind>
  /** null = tüm topluluklar */
  community: number | null
}

/** Filtreye uyan düğümler için maske (çizim ve liste aynı maskeyi kullanır). */
export function visibleMask(nodes: readonly SnapshotNode[], f: Filter): Uint8Array {
  const out = new Uint8Array(nodes.length)
  nodes.forEach((n, i) => {
    out[i] = f.kinds.has(n.kind) && (f.community === null || n.community === f.community) ? 1 : 0
  })
  return out
}

/** Dosya kümesine ait düğümlerin dizinleri (dosya düğümü ve içindeki semboller). */
export function nodesForFiles(nodes: readonly SnapshotNode[], files: Iterable<string>): Set<number> {
  const want = new Set<string>()
  for (const f of files) want.add(normalizePath(f))
  const out = new Set<number>()
  if (want.size === 0) return out
  nodes.forEach((n, i) => {
    if (n.file !== null && want.has(normalizePath(n.file))) out.add(i)
  })
  return out
}

export function normalizePath(p: string): string {
  return p.replace(/\\/g, '/').replace(/^\.\//, '')
}

/** Kimlik → dizin. */
export function indexById(s: Snapshot): Map<string, number> {
  return new Map(s.nodes.map((n, i) => [n.id, i]))
}

/** Yol adımlarını çizimdeki dizin çiftlerine çevirir; grafikte olmayan adım çizgiyi böler. */
export function pathSegments(steps: readonly PathStep[], index: ReadonlyMap<string, number>): Array<[number, number]> {
  const out: Array<[number, number]> = []
  for (let i = 1; i < steps.length; i++) {
    const a = index.get(steps[i - 1]!.node.id)
    const b = index.get(steps[i]!.node.id)
    if (a !== undefined && b !== undefined) out.push([a, b])
  }
  return out
}

/** [Ajana ver]: mesaj kutusuna eklenecek başvuru. Dosya `@yol` ile, sembol adı ve satırıyla birlikte. */
export function referenceFor(n: Pick<NodeRef, 'id' | 'kind' | 'name' | 'file' | 'line'>): string {
  const kind = KIND_LABEL[n.kind].toLowerCase()
  if (n.file === null) return `Graf düğümü \`${n.id}\` (${kind})`
  const file = normalizePath(n.file)
  if (n.kind === 'module') return `@${file}`
  return n.line !== null ? `@${file} — \`${n.name}\` (${kind}, satır ${n.line})` : `@${file} — \`${n.name}\` (${kind})`
}

/** Birlikte değişme oranlarının en yükseği (0..1); yoksa null. */
export function topCoChange(e: Pick<Explanation, 'co_changes'>): { file: string; ratio: number } | null {
  let best: { file: string; ratio: number } | null = null
  for (const l of e.co_changes) {
    const r = l.kind.ratio ?? 0
    if (best === null || r > best.ratio) best = { file: l.node.file ?? l.node.name, ratio: r }
  }
  return best
}

/** "3 sn önce", "5 dk önce", "2 sa önce", yoksa tarih. */
export function ago(unixSecs: number | null, nowMs: number): string {
  if (unixSecs === null) return 'hiç'
  const s = Math.max(0, Math.round(nowMs / 1000 - unixSecs))
  if (s < 86_400) {
    const intl = agoIntl(s)
    if (intl) return intl
  }
  if (s < 60) return `${s} sn önce`
  if (s < 3600) return `${Math.floor(s / 60)} dk önce`
  if (s < 86_400) return `${Math.floor(s / 3600)} sa önce`
  return new Date(unixSecs * 1000).toLocaleDateString(uiLocale())
}

/** Çizimde adı yazılacak düğümler: en yüksek dereceli `count` görünür düğüm. */
export function labelled(nodes: readonly SnapshotNode[], mask: Uint8Array, count: number): Set<number> {
  const idx = nodes.map((_, i) => i).filter((i) => mask[i])
  idx.sort((a, b) => nodes[b]!.degree - nodes[a]!.degree)
  return new Set(idx.slice(0, count))
}

/** Düğüm yarıçapı (dünya biriminde): derece ile yavaş büyür. */
export function radius(n: Pick<SnapshotNode, 'degree' | 'kind'>): number {
  return Math.min(14, 3 + Math.sqrt(n.degree) * 1.2) + (n.kind === 'package' ? 3 : 0)
}
