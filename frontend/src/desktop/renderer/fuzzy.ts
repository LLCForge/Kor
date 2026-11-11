// Dosya ağacı araması için basit bulanık eşleme: sorgu karakterleri yolda sırayla geçmeli.
// Puan: ardışık eşleşme, segment başı (/, -, _, ., büyük harf) ve dosya adında eşleşme öne çıkar.

const BOUNDARY = /[/\-_. ]/

function scoreIn(query: string, target: string): number | null {
  const lower = target.toLowerCase()
  let score = 0
  let ti = 0
  let prev = -2
  for (const ch of query) {
    const found = lower.indexOf(ch, ti)
    if (found < 0) return null
    score += 1
    if (found === prev + 1) score += 5
    const before = target[found - 1]
    const at = target[found] ?? ''
    if (found === 0 || (before !== undefined && BOUNDARY.test(before)) || (at !== at.toLowerCase() && before === before?.toLowerCase())) score += 8
    prev = found
    ti = found + 1
  }
  // Eşit puanda kısa yol öne çıkar.
  return score - target.length * 0.01
}

/** null: eşleşme yok. Büyük puan daha iyi. */
export function fuzzyScore(query: string, path: string): number | null {
  const q = query.trim().toLowerCase().replace(/\s+/g, '')
  if (!q) return 0
  const slash = path.lastIndexOf('/')
  const name = path.slice(slash + 1)
  const inName = scoreIn(q, name)
  if (inName !== null) return inName + 20
  return scoreIn(q, path)
}

export function fuzzyFilter(paths: readonly string[], query: string, limit = 300): string[] {
  const scored: { path: string; score: number }[] = []
  for (const path of paths) {
    const score = fuzzyScore(query, path)
    if (score !== null) scored.push({ path, score })
  }
  return scored
    .sort((a, b) => b.score - a.score || a.path.localeCompare(b.path))
    .slice(0, limit)
    .map((s) => s.path)
}

export interface FilterNode {
  name: string
  path: string
  isDir: boolean
  children: FilterNode[]
}

/** Eşleşen yollardan klasör ağacı kurar; klasörler önce, sonra ada göre. */
export function buildTree(paths: readonly string[]): FilterNode[] {
  const root: FilterNode = { name: '', path: '', isDir: true, children: [] }
  for (const p of paths) {
    let node = root
    const parts = p.split('/')
    parts.forEach((part, i) => {
      const isDir = i < parts.length - 1
      const childPath = parts.slice(0, i + 1).join('/')
      let child = node.children.find((c) => c.name === part && c.isDir === isDir)
      if (!child) {
        child = { name: part, path: childPath, isDir, children: [] }
        node.children.push(child)
      }
      node = child
    })
  }
  const sort = (nodes: FilterNode[]) => {
    nodes.sort((a, b) => Number(b.isDir) - Number(a.isDir) || a.name.localeCompare(b.name, 'tr'))
    for (const n of nodes) sort(n.children)
  }
  sort(root.children)
  return root.children
}
