// ipc.ts `offline` ad alanını tiplemiyor; tipler ipc-offline.ts içinde.
import type { OfflineApi } from '../../../ipc-offline.ts'
import { api } from '../../api.ts'

export const offlineApi = (api as unknown as { offline: OfflineApi }).offline
