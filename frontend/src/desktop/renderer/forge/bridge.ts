// ipc.ts `forge` ad alanını tiplemiyor; tipler ipc-forge.ts içinde, erişim burada tek yerde.
import type { ForgeApi } from '../../ipc-forge.ts'
import { api } from '../api.ts'

export const forgeApi = (api as unknown as { forge: ForgeApi }).forge
