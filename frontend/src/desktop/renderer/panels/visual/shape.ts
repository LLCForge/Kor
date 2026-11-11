// Dosya bazında onay yanıtını (ReviewPanel) yazılacak dosya listesine çevirir (ADR 0030 + 0048).
import type { Approval } from '../../../../core/agent.ts'
import type { FileDecisions, ReviewFile } from '../../../ipc-review.ts'

export interface WriteFile {
  path: string
  content: string
}

/** 'no' hiçbirini yazmaz; karar verilmeyen dosya onayı izler; düzenlenen dosya düzenlenmiş içerikle yazılır. */
export function toWrite(files: readonly ReviewFile[], approval: Approval, decisions?: FileDecisions): WriteFile[] {
  if (approval === 'no') return []
  const out: WriteFile[] = []
  for (const f of files) {
    const d = decisions?.[f.path] ?? 'accept'
    if (d === 'reject') continue
    const content = typeof d === 'object' ? d.edit : f.after
    if (content !== null) out.push({ path: f.path, content })
  }
  return out
}

/** `12.3 MB` */
export function mb(bytes: number): string {
  return `${(bytes / 1_048_576).toFixed(1)} MB`
}
