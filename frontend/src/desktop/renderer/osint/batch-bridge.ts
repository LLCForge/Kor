// ipc.ts `korfolder` ve `batch` ad alanlarını tiplemiyor; tipler ipc-korfolder.ts ve ipc-batch.ts içinde.
import type { BatchApi } from '../../ipc-batch.ts'
import type { KorFolderApi } from '../../ipc-korfolder.ts'
import { api } from '../api.ts'

export const korfolderApi = (api as unknown as { korfolder: KorFolderApi }).korfolder
export const batchApi = (api as unknown as { batch: BatchApi }).batch

/** Toplu düzenleme penceresini açar (bağlam menüsü, `kor://edit`, komut paleti). */
export function openBatchEdit(files: readonly string[]): void {
  window.dispatchEvent(new CustomEvent('osint:edit', { detail: { files: [...files] } }))
}

/** Komut paleti: "Kor ile düzenle…" — dosya seçtirir, sonra pencereyi açar. */
export async function pickAndBatchEdit(): Promise<void> {
  const files = await api.pickFiles()
  if (files.length > 0) openBatchEdit(files)
}
