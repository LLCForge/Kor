// ipc.ts `osextra` ad alanını tiplemiyor; tipler ipc-osextra.ts içinde, erişim tek yerde.
import type { OsExtraApi } from '../../ipc-osextra.ts'
import { api } from '../api.ts'

export const osextraApi = (api as unknown as { osextra: OsExtraApi }).osextra
