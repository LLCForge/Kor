// §27.3 birleştirme: dosya bazında inceleme yanıtı (ReviewPanel) → yazılacak dosyalar.
import type { Approval } from '../../../../core/agent.ts'
import type { FileDecisions, ReviewFile } from '../../../ipc-review.ts'
import type { ForkNode } from '../../../ipc-ek14.ts'

/** `no` hiçbirini, `yes` (kararsız) hepsini; kararlarda reddedilen atlanır, düzenlenen onun içeriğiyle yazılır. */
export function acceptedFiles(files: readonly ReviewFile[], approval: Approval, decisions?: FileDecisions): { path: string; content: string | null }[] {
  if (approval === 'no') return []
  const out: { path: string; content: string | null }[] = []
  for (const f of files) {
    const d = decisions?.[f.path] ?? 'accept'
    if (d === 'reject') continue
    out.push({ path: f.path, content: typeof d === 'object' ? d.edit : f.after })
  }
  return out
}

/** Ağacı düz satırlara çevirir (girinti derinliği ile), görünüm ve seçim için. */
export function flatten(nodes: readonly ForkNode[], depth = 0): { node: ForkNode; depth: number }[] {
  return nodes.flatMap((n) => [{ node: n, depth }, ...flatten(n.children, depth + 1)])
}
