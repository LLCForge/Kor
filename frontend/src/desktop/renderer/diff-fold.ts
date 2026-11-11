// Diff'te uzun değişmemiş bağlam bloklarını katlar (cdesktop/@git-diff-view "118 unmodified lines ⌄").
// Saf; DiffView hem birleşik hem yan yana satırlar için kullanır.
export type FoldSegment<T> = { kind: 'rows'; rows: T[] } | { kind: 'fold'; rows: T[]; count: number }

/**
 * Bağlam satırı dizileri `min` satırdan uzunsa, değişikliklere komşu `keep` satır görünür kalır, ortası katlanır.
 * Dosya başı/sonundaki bağlamda yalnızca değişikliğe bakan taraf tutulur.
 */
export function foldContext<T>(rows: readonly T[], isCtx: (row: T) => boolean, keep = 3, min = 8): FoldSegment<T>[] {
  const out: FoldSegment<T>[] = []
  let i = 0
  const pushRows = (list: T[]) => {
    if (list.length === 0) return
    const last = out.at(-1)
    if (last?.kind === 'rows') last.rows.push(...list)
    else out.push({ kind: 'rows', rows: list })
  }
  while (i < rows.length) {
    if (!isCtx(rows[i]!)) {
      pushRows([rows[i]!])
      i++
      continue
    }
    let j = i
    while (j < rows.length && isCtx(rows[j]!)) j++
    const run = rows.slice(i, j)
    const atStart = i === 0
    const atEnd = j === rows.length
    const head = atStart ? 0 : keep
    const tail = atEnd ? 0 : keep
    if (run.length > min && run.length - head - tail >= 2) {
      pushRows(run.slice(0, head))
      const middle = run.slice(head, run.length - tail)
      out.push({ kind: 'fold', rows: middle, count: middle.length })
      pushRows(run.slice(run.length - tail))
    } else {
      pushRows(run)
    }
    i = j
  }
  return out
}
