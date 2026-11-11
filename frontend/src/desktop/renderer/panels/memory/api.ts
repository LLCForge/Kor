// Hafıza köprüleri: KorApi `memory` / `projmem` ad alanlarını tiplemez (ipc.ts'e dokunulmaz), tipli erişim burada.
import type { MemoryApi, ProjMemApi } from '../../../ipc-memory.ts'
import { api } from '../../api.ts'

const typed = api as unknown as { memory: MemoryApi; projmem: ProjMemApi }

export const memoryApi = typed.memory
export const projmemApi = typed.projmem
