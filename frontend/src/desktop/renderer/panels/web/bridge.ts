// ipc.ts `web` ad alanını tiplemiyor; tipler ipc-web.ts içinde, erişim tek yerde.
import type { WebApi } from '../../../ipc-web.ts'
import { api } from '../../api.ts'

export const webApi = (api as unknown as { web: WebApi }).web
