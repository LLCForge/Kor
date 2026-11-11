// ipc.ts `remote` ad alanını tiplemiyor; tipler ipc-remote.ts içinde.
import type { RemoteApi } from '../../../ipc-remote.ts'
import { api } from '../../api.ts'

export const remoteApi = (api as unknown as { remote: RemoteApi }).remote
