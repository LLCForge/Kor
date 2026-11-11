import { diffWordsWithSpace, type Change } from 'diff'

export type LineKind = 'ctx' | 'add' | 'del'

/** Kelime düzeyi parça: changed=true olanlar koyu vurgulanır (cdesktop/@git-diff-view). */
export interface WordSeg {
  text: string
  changed: boolean
}

export interface UnifiedRow {
  oldNo: number | null
  newNo: number | null
  text: string
  kind: LineKind
  /** Eşleşen bir karşı satır varsa kelime parçaları */
  segs?: WordSeg[]
}

export interface SplitCell {
  no: number
  text: string
  kind: LineKind
  segs?: WordSeg[]
}

const MAX_WORD_DIFF_CHARS = 2000

/** Değişen satır çifti için kelime parçaları; çok uzun satırlarda (performans) ya da tamamen farklıysa atlanır. */
export function wordSegments(del: string, add: string): { del: WordSeg[]; add: WordSeg[] } | null {
  if (del.length > MAX_WORD_DIFF_CHARS || add.length > MAX_WORD_DIFF_CHARS) return null
  const parts = diffWordsWithSpace(del, add)
  const d: WordSeg[] = []
  const a: WordSeg[] = []
  let same = 0
  for (const p of parts) {
    if (p.added) a.push({ text: p.value, changed: true })
    else if (p.removed) d.push({ text: p.value, changed: true })
    else {
      d.push({ text: p.value, changed: false })
      a.push({ text: p.value, changed: false })
      same += p.value.trim().length
    }
  }
  // Ortak metin yoksa vurgu anlamsız: satırın tamamı değişmiş.
  if (same === 0) return null
  return { del: d, add: a }
}

/** Yan yana görünüm satırı; boş taraf null (karşı tarafta eklenen/silinen satır). */
export interface SplitRow {
  left: SplitCell | null
  right: SplitCell | null
}

export function toLines(value: string): string[] {
  if (value === '') return []
  return value.replace(/\r?\n$/, '').split(/\r?\n/)
}

export function unifiedRows(parts: readonly Change[]): UnifiedRow[] {
  const rows: UnifiedRow[] = []
  let oldNo = 0
  let newNo = 0
  let i = 0
  while (i < parts.length) {
    const part = parts[i]
    if (!part) break
    if (!part.added && !part.removed) {
      for (const text of toLines(part.value)) rows.push({ oldNo: ++oldNo, newNo: ++newNo, text, kind: 'ctx' })
      i++
      continue
    }
    const dels: string[] = []
    const adds: string[] = []
    for (; i < parts.length; i++) {
      const p = parts[i]
      if (!p || (!p.added && !p.removed)) break
      if (p.removed) dels.push(...toLines(p.value))
      else adds.push(...toLines(p.value))
    }
    const delRows: UnifiedRow[] = dels.map((text) => ({ oldNo: ++oldNo, newNo: null, text, kind: 'del' }))
    const addRows: UnifiedRow[] = adds.map((text) => ({ oldNo: null, newNo: ++newNo, text, kind: 'add' }))
    for (let k = 0; k < Math.min(delRows.length, addRows.length); k++) {
      const ws = wordSegments(delRows[k]!.text, addRows[k]!.text)
      if (ws) {
        delRows[k]!.segs = ws.del
        addRows[k]!.segs = ws.add
      }
    }
    rows.push(...delRows, ...addRows)
  }
  return rows
}

/**
 * Ardışık silme/ekleme bloklarını satır satır eşler: değiştirilen satırlar yan yana,
 * fazla kalanlar karşısı boş olarak gösterilir. Bağlam satırları iki tarafta da yer alır.
 */
export function splitRows(parts: readonly Change[]): SplitRow[] {
  const rows: SplitRow[] = []
  let oldNo = 0
  let newNo = 0
  let i = 0
  while (i < parts.length) {
    const part = parts[i]
    if (!part) break
    if (!part.added && !part.removed) {
      for (const text of toLines(part.value)) rows.push({ left: { no: ++oldNo, text, kind: 'ctx' }, right: { no: ++newNo, text, kind: 'ctx' } })
      i++
      continue
    }
    const dels: string[] = []
    const adds: string[] = []
    for (; i < parts.length; i++) {
      const p = parts[i]
      if (!p || (!p.added && !p.removed)) break
      if (p.removed) dels.push(...toLines(p.value))
      else adds.push(...toLines(p.value))
    }
    for (let k = 0; k < Math.max(dels.length, adds.length); k++) {
      const del = dels[k]
      const add = adds[k]
      const ws = del !== undefined && add !== undefined ? wordSegments(del, add) : null
      rows.push({
        left: del === undefined ? null : { no: ++oldNo, text: del, kind: 'del', ...(ws ? { segs: ws.del } : {}) },
        right: add === undefined ? null : { no: ++newNo, text: add, kind: 'add', ...(ws ? { segs: ws.add } : {}) },
      })
    }
  }
  return rows
}
