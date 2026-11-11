import type { Approval } from '../../../core/agent.ts'
import type { FileDecisions } from '../../ipc-review.ts'
import { api } from '../api.ts'

/** `approve` + isteğe bağlı dosya kararları (ipc.ts'deki tip iki argümanlıdır; köprü üçüncüyü Rust'a iletir). */
export function approveWithFiles(id: string, approval: Approval, files?: FileDecisions): Promise<void> {
  const typed = api as unknown as { approve(id: string, approval: Approval, files?: FileDecisions): Promise<void> }
  return files ? typed.approve(id, approval, files) : typed.approve(id, approval)
}
