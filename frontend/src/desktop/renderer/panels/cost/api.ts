// Maliyet/Bağlam/Bench köprüleri: KorApi bu ad alanlarını tiplemez (ipc.ts'e dokunulmaz), tipli erişim burada.
import { useEffect } from 'react'
import type { BenchApi, ContextApi, CostApi, PackagesCostApi } from '../../../ipc-cost.ts'
import { api } from '../../api.ts'

const typed = api as unknown as { cost: CostApi; context: ContextApi; bench: BenchApi; packages: PackagesCostApi }

export const costApi = typed.cost
export const contextApi = typed.context
export const benchApi = typed.bench
export const packagesApi = typed.packages

/** Bu panellerin SWR anahtarları `['cost:…', …]` / `['context:…', …]` biçimindedir. */
export const keyPrefix = (prefix: string) => (k: unknown): boolean => Array.isArray(k) && typeof k[0] === 'string' && k[0].startsWith(prefix)

/**
 * Bir tur bitince (`kor:event` → `done`) yeniden yükler. Defter her model çağrısında yazılır;
 * tur sonu olayı hangi sohbetten gelirse gelsin pano güncellenir.
 */
export function useRefreshOnTurnDone(refresh: () => void): void {
  useEffect(
    () =>
      api.onEvent((event) => {
        if (event.type === 'done') refresh()
      }),
    [refresh],
  )
}
